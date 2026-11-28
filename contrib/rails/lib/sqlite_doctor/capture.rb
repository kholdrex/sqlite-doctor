# Runs pending ActiveRecord migrations against a copy of a SQLite database and
# writes one SQL file for `sqlite-doctor`: the baseline schema, the
# connection's PRAGMA foreign_keys state, and the SQL ActiveRecord reported
# while migrating, including transaction boundaries.
#
# ActiveRecord::Base is pointed at the copy for the run and restored
# afterwards. A migration or model that opens its own connection to the
# original database is not redirected; the capture fails if another connection
# committed to the original while migrating. A relative database path is
# resolved like ActiveRecord does: against Rails.root when Rails is loaded.
#
# Standalone:
#   ruby capture.rb DATABASE MIGRATIONS_DIR OUTPUT
# In a Rails app, see lib/tasks/sqlite_doctor.rake.

require "active_record"
require "sqlite3"
require "tempfile"
require "tmpdir"

module SqliteDoctor
  # Names the migration whose SQL is being recorded, for the directives that
  # point findings at their migration file.
  # Acts only while a capture runs on this thread. Thread variables, unlike
  # Thread.current[], are shared with fibers the migration starts.
  module LabelMigrations
    def self.capturing?
      Thread.current.thread_variable_get(:sqlite_doctor_capturing)
    end

    def self.current
      Thread.current.thread_variable_get(:sqlite_doctor_migration)
    end

    def self.labelled(migration)
      previous = current
      name = migration.respond_to?(:filename) ? migration.filename : migration.name
      Thread.current.thread_variable_set(:sqlite_doctor_migration, name)
      yield
    ensure
      Thread.current.thread_variable_set(:sqlite_doctor_migration, previous)
    end

    private

    def execute_migration_in_transaction(migration)
      return super unless SqliteDoctor::LabelMigrations.capturing?

      SqliteDoctor::LabelMigrations.labelled(migration) { super }
    end
  end
end

module SqliteDoctor::Capture
  SIDE_FILES = ["", "-wal", "-shm", "-journal"].freeze
  CAPTURE_HEADER = "-- sqlite-doctor capture:".freeze
  # A line sqlite-doctor could read as a directive. Copied SQL is never
  # rewritten, because the same text can sit inside a quoted name.
  RESERVED_LINE = /^[ \t\f\r\u{feff}]*-- sqlite-doctor:/

  # A capture that cannot run safely or cannot see what it needs.
  class Refused < RuntimeError; end

  module_function

  # name and env_name keep the selected database's identity on the copy.
  # watch: another file that must not change while migrating, such as the
  # configured database when the capture runs on a substitute.
  def capture(config:, migrations_paths:, output:, root: nil, name: nil, env_name: nil, watch: nil)
    database = resolve_database_path(config.fetch(:database).to_s)
    raise ArgumentError, "#{database} does not exist" unless File.file?(database)
    database = File.realpath(database)
    output = File.expand_path(output)
    output = File.join(File.realpath(File.dirname(output)), File.basename(output))
    reject_output_aliasing(database, output)
    reject_other_pools(database)

    reject_in_memory_connection
    previous_config = current_db_config
    Dir.mktmpdir("sqlite-doctor") do |dir|
      copy = File.join(dir, File.basename(database))
      SQLite3::Database.new(database, readonly: true) do |db|
        db.execute("VACUUM INTO ?", [copy])
      end

      begin
        ActiveRecord::Base.establish_connection(named_config(config.merge(database: copy), name, env_name))
        copy_pool = ActiveRecord::Base.connection_pool
        connection = base_connection
        unless main_database_path(connection) == File.realpath(copy)
          raise Refused, "refusing to migrate: the connection is not attached to the copy"
        end

        schema = connection.select_values(<<~SQL)
          SELECT sql FROM sqlite_schema
          WHERE sql IS NOT NULL AND name NOT GLOB 'sqlite_*'
          ORDER BY rowid
        SQL
        foreign_keys = connection.select_value("PRAGMA foreign_keys")
        rows_before = row_counts(connection)

        # data_version changes when another connection commits to the file,
        # and not when WAL frames are checkpointed (except TRUNCATE).
        watchers = [database, watch].compact.uniq.map do |path|
          watcher = SQLite3::Database.new(path, readonly: true)
          [path, watcher, watcher.get_first_value("PRAGMA data_version")]
        end
        run = run_migrations(migrations_paths, copy_pool, connection)
        statements = run[:statements]
        failures = run[:failures]
        error = run[:error]
        rows_after = row_counts(base_connection)
        watchers.each do |path, watcher, version|
          next if watcher.get_first_value("PRAGMA data_version") == version

          raise Refused, "#{path} changed while migrating the copy: a migration, model or " \
                         "other process committed to it (or ran a TRUNCATE checkpoint)"
        end
      ensure
        watchers&.each { |_, watcher, _| watcher.close }
        restore_connection(previous_config)
      end

      shrunk = rows_before.filter_map do |table, before|
        after = rows_after[table]
        "#{single_line(table)} #{before} -> #{after}" if after && after < before
      end
      incomplete = []
      # The class only: messages such as StatementInvalid's quote SQL, which
      # can hold data, and this reason is printed.
      incomplete << "the migration raised #{error.class}" if error
      if failures.any?
        incomplete << "#{failures.size} statement(s) failed; a failure can roll back earlier statements"
      end
      incomplete << "migration SQL ran on a second connection to the copy" if run[:other_connection]
      incomplete << "migration SQL ran on another database, which the capture does not record" if run[:other_database]
      if run[:reconnected]
        incomplete << "the migrating connection reconnected, which resets PRAGMA foreign_keys without a trace"
      end
      copied = schema + statements.reject { |_, failed| failed }.map(&:first)
      if copied.any? { |sql| sql.match?(RESERVED_LINE) }
        incomplete << "copied SQL has a line that starts with `-- sqlite-doctor:`"
      end
      write_atomically(output, render(schema, foreign_keys, statements, failures, shrunk, error, incomplete, root))
      { output: output, statements: statements.size - failures.size, failed: failures.size,
        pending: run[:pending]&.map { |name| relative_label(name, root) },
        shrunk_tables: shrunk, error: error, incomplete: incomplete }
    end
  end

  # Records the SQL the migrating connection ran on the copy. SQL from
  # another connection to the copy has its own transaction and PRAGMA state,
  # so it is left out and the capture is marked incomplete.
  # Rails sets PRAGMA foreign_keys on the raw connection whenever it
  # reconnects, outside the notifications, so a reconnect is recorded.
  # SQL this thread sends to any other database is not recorded either, so
  # it also makes the capture incomplete. Migration announcements are
  # suppressed: they print SQL, which can hold data.
  def run_migrations(migrations_paths, copy_pool, migrating_connection)
    statements = []
    failures = []
    other_connection = false
    other_database = false
    reconnected = false
    pending = nil
    watch_reconnects = Module.new do
      define_method(:configure_connection) do |*arguments, **options, &block|
        reconnected = true
        super(*arguments, **options, &block)
      end
      private :configure_connection
    end
    migrating_connection.singleton_class.prepend(watch_reconnects)
    ActiveRecord::Migrator.prepend(SqliteDoctor::LabelMigrations) unless ActiveRecord::Migrator < SqliteDoctor::LabelMigrations
    subscription = ActiveSupport::Notifications.subscribe("sql.active_record") do |*, payload|
      next if payload[:cached]

      # From any thread: a migration may hand work to a thread of its own.
      unless payload[:connection]&.pool.equal?(copy_pool)
        other_database = true
        next
      end

      unless payload[:connection].equal?(migrating_connection)
        other_connection = true
        next
      end

      # A failed statement did not change the schema, so it must not count as
      # restoring an object. Keep it visible as a comment.
      failures << payload[:sql] if payload[:exception]
      statements << [payload[:sql], !payload[:exception].nil?, SqliteDoctor::LabelMigrations.current]
    end
    Thread.current.thread_variable_set(:sqlite_doctor_capturing, true)
    begin
      context = ActiveRecord::MigrationContext.new(Array(migrations_paths))
      pending = context.open.pending_migrations.map(&:filename)
      ActiveRecord::Migration.suppress_messages { context.migrate }
      error = nil
    rescue StandardError => e
      error = e
    ensure
      Thread.current.thread_variable_set(:sqlite_doctor_capturing, nil)
      ActiveSupport::Notifications.unsubscribe(subscription)
    end
    { statements: statements, failures: failures, error: error, other_connection: other_connection,
      other_database: other_database, reconnected: reconnected, pending: pending }
  end

  # Switching ActiveRecord::Base to the copy and back reconnects, which
  # discards an in-memory database the caller is using.
  def reject_in_memory_connection
    previous_config = current_db_config
    return unless previous_config

    in_memory = pool_database_path(previous_config).nil?
    # A connection opened with SQLite's MEMORY flag has a file-like setting.
    in_memory ||= ActiveRecord::Base.connection_pool.connected? && main_database_path(base_connection).nil?
    return unless in_memory

    raise Refused, "the current connection uses an in-memory database, which switching connections would " \
                   "discard; run against a file-backed configuration"
  end

  def named_config(config, name, env_name)
    return config unless name

    ActiveRecord::DatabaseConfigurations::HashConfig.new(env_name || "default_env", name, config)
  end

  def restore_connection(previous_config)
    begin
      ActiveRecord::Base.connection_pool.disconnect!
    rescue ActiveRecord::ConnectionNotEstablished
      nil
    end
    if previous_config
      ActiveRecord::Base.establish_connection(previous_config)
    else
      ActiveRecord::Base.remove_connection
    end
  end

  def resolve_database_path(path)
    root = defined?(Rails) && Rails.respond_to?(:root) && Rails.root
    File.expand_path(path, root || Dir.pwd)
  end

  def current_db_config
    ActiveRecord::Base.connection_db_config
  rescue ActiveRecord::ConnectionNotEstablished
    nil
  end

  def base_connection
    if ActiveRecord::Base.respond_to?(:lease_connection)
      ActiveRecord::Base.lease_connection
    else
      ActiveRecord::Base.connection
    end
  end

  def main_database_path(connection)
    main = connection.select_rows("PRAGMA database_list").find { |row| row[1] == "main" }
    main && !main[2].to_s.empty? ? File.realpath(main[2]) : nil
  end

  # Names are compared without case, which some file systems ignore.
  def reject_output_aliasing(database, output)
    if output.match?(/-(wal|shm|journal)\z/i)
      raise ArgumentError, "refusing to write the capture to #{output}: " \
                           "SQLite uses names ending in -wal, -shm and -journal"
    end

    protected_databases(database).each do |path|
      next unless output.casecmp?(path) ||
                  (File.exist?(output) && File.exist?(path) && File.identical?(output, path))

      raise ArgumentError, "refusing to write the capture over #{path}"
    end
  end

  # The database being migrated and every file the app configures a database
  # at or has a connection pool on, including pools that are not connected
  # yet. Adapter names are not checked: an alias can still open SQLite. A
  # `file:` setting counts both as a URI and as a plain file name, because
  # older sqlite3 gems open it as a plain name.
  def protected_databases(database)
    configs = ActiveRecord::Base.configurations.configs_for(include_hidden: true)
    pools = ActiveRecord::Base.connection_handler.connection_pool_list(:all).map(&:db_config)
    paths = (configs + pools).flat_map do |db_config|
      setting = db_config.database.to_s
      names = [pool_database_path(db_config)]
      names << resolve_database_path(setting) if setting.start_with?("file:")
      names.compact.flat_map do |path|
        directory = File.dirname(path)
        path = File.join(File.realpath(directory), File.basename(path)) if File.directory?(directory)
        File.exist?(path) ? [path, File.realpath(path)] : [path]
      end
    end
    [database, *paths].uniq
  end

  def reject_other_pools(database)
    base_pool = current_db_config && ActiveRecord::Base.connection_pool
    ActiveRecord::Base.connection_handler.connection_pool_list(:all).each do |pool|
      next if pool.equal?(base_pool)

      path = pool_database_path(pool.db_config)
      next unless path && File.exist?(path) && File.identical?(path, database)

      raise Refused, "refusing to migrate: connection pool #{pool.db_config.name} also uses #{database}"
    end
  end

  # The file a pool's database setting names; nil for an in-memory database.
  def pool_database_path(db_config)
    path = db_config.database.to_s
    return nil if path.empty? || path == ":memory:"
    return resolve_database_path(path) unless path.start_with?("file:")

    location, query = path.delete_prefix("file:").sub(/#.*\z/m, "").split("?", 2)
    location = location.to_s
    if location.start_with?("//")
      authority, location = location[2..].split("/", 2)
      unless ["", "localhost"].include?(authority.to_s)
        raise Refused, "refusing to migrate: cannot tell which file connection pool " \
              "#{db_config.name} uses (#{path})"
      end
      location = "/#{location}"
    end
    unescape = ->(text) { text.gsub(/%(\h\h)/) { Regexp.last_match(1).hex.chr } }
    location = unescape.call(location)
    return nil if location.empty? || location == ":memory:"
    # With more than one mode parameter the file may still be opened.
    modes = query.to_s.split("&").map(&unescape).select { |parameter| parameter.start_with?("mode=") }
    return nil if modes == ["mode=memory"]

    # SQLite resolves a relative URI path against the working directory.
    File.expand_path(location)
  end

  def row_counts(connection)
    tables = connection.select_values(<<~SQL)
      SELECT name FROM sqlite_schema
      WHERE type = 'table' AND name NOT GLOB 'sqlite_*'
    SQL
    tables.to_h do |table|
      [table, connection.select_value("SELECT count(*) FROM #{connection.quote_table_name(table)}")]
    end
  end

  def render(schema, foreign_keys, statements, failures, shrunk, error, incomplete, root = nil)
    lines = []
    lines << "#{CAPTURE_HEADER} ActiveRecord #{ActiveRecord::VERSION::STRING}, " \
             "SQLite #{SQLite3::SQLITE_VERSION}"
    lines << "-- Migration target: temporary database copy."
    lines << "-- Migration failed: #{single_line("#{error.class}: #{error.message.lines.first&.strip}")}" if error
    incomplete.each { |reason| lines << "-- sqlite-doctor:incomplete #{reason}" }
    lines << "-- #{failures.size} statement(s) failed and are kept below as comments." if failures.any?
    shrunk.each { |line| lines << "-- Observed on the copy: rows in #{line}" }
    lines << ""
    lines << "-- Baseline schema"
    lines << "-- sqlite-doctor:migration schema before migrating"
    schema.each { |sql| lines << terminate(sql) }
    lines << ""
    lines << "-- Connection state before migrating"
    lines << "-- sqlite-doctor:synthetic connection state before migrating"
    lines << "PRAGMA foreign_keys = #{foreign_keys};"
    lines << ""
    lines << "-- Executed statements"
    current = :none
    statements.each do |sql, failed, label|
      if label != current
        lines << (label ? "-- sqlite-doctor:migration #{relative_label(label, root)}" : "-- sqlite-doctor:synthetic ActiveRecord migrator")
        current = label
      end
      lines << (failed ? "-- failed: #{sql.gsub(/\s+/, ' ')}" : terminate(sql))
    end
    lines.join("\n") + "\n"
  end

  def single_line(text)
    text.gsub(/[[:cntrl:]]+/, " ")
  end

  # A migration file relative to the application root, on one line.
  def relative_label(path, root = nil)
    root ||= defined?(Rails) && Rails.respond_to?(:root) && Rails.root ? Rails.root.to_s : Dir.pwd
    root = File.realpath(root) if File.directory?(root)
    path = File.expand_path(path, root)
    path = File.realpath(path) if File.exist?(path)
    path = path.delete_prefix("#{root}/") if path.start_with?("#{root}/")
    single_line(path)
  end

  # SQLite ends a comment at the end of each statement it is given, but here
  # the statements are joined. After a newline, `/**/` closes a block comment
  # left open and is an empty comment otherwise, so the semicolon always ends
  # the statement.
  def terminate(sql)
    sql = sql.sub(/;?\s*\z/, "")
    sql.include?("--") || sql.include?("/*") ? "#{sql}\n/**/;" : "#{sql};"
  end

  # Tempfile creates the staging file with O_EXCL and mode 0600, so a file or
  # symlink already at that name is never followed or truncated, and removes
  # it afterwards. A new capture is published with a hard link, which fails if
  # anything appeared at OUTPUT meanwhile. Only a regular file holding an
  # earlier capture is replaced, so a database a migration created there or
  # one opened under another name is refused. A replaced capture keeps its
  # group and mode when the group can be kept; the SQL can hold literal data
  # from data migrations.
  def write_atomically(output, content)
    Tempfile.create([File.basename(output), ".tmp"], File.dirname(output)) do |staging|
      staging.write(content)
      staging.close
      if File.symlink?(output)
        raise ArgumentError, "refusing to write the capture over #{output}: it is a symbolic link"
      elsif File.exist?(output)
        unless File.file?(output) &&
               File.open(output, "rb") { |file| file.read(CAPTURE_HEADER.bytesize) } == CAPTURE_HEADER
          raise ArgumentError, "refusing to write the capture over #{output}: it is not an earlier capture"
        end

        stat = File.stat(output)
        begin
          File.chown(nil, stat.gid, staging.path)
          File.chmod(stat.mode & 0o777, staging.path)
        rescue Errno::EPERM
          nil
        end
        File.rename(staging.path, output)
      else
        begin
          File.link(staging.path, output)
        rescue Errno::EEXIST
          raise ArgumentError, "refusing to write the capture over #{output}: it appeared while migrating"
        end
      end
    end
  end
end

if $PROGRAM_NAME == __FILE__
  database, migrations, output = ARGV
  abort "usage: ruby #{File.basename(__FILE__)} DATABASE MIGRATIONS_DIR OUTPUT" unless output

  result = SqliteDoctor::Capture.capture(
    config: { adapter: "sqlite3", database: database },
    migrations_paths: migrations,
    output: output
  )
  warn "wrote #{result[:statements]} statements to #{output}"
  result[:shrunk_tables].each { |line| warn "rows lost on the copy: #{line}" }
  if result[:error]
    warn "migration failed: #{result[:error].class}: #{result[:error].message}"
    exit 1
  end
end

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
#   ruby sqlite_doctor_capture.rb DATABASE MIGRATIONS_DIR OUTPUT
# In a Rails app, see sqlite_doctor.rake.

require "active_record"
require "sqlite3"
require "tempfile"
require "tmpdir"

module SqliteDoctorCapture
  SIDE_FILES = ["", "-wal", "-shm", "-journal"].freeze
  CAPTURE_HEADER = "-- sqlite-doctor capture:".freeze

  module_function

  def capture(config:, migrations_paths:, output:)
    database = resolve_database_path(config.fetch(:database).to_s)
    raise ArgumentError, "#{database} does not exist" unless File.file?(database)
    database = File.realpath(database)
    output = File.expand_path(output)
    output = File.join(File.realpath(File.dirname(output)), File.basename(output))
    reject_output_aliasing(database, output)
    reject_other_pools(database)

    previous_config = current_db_config
    Dir.mktmpdir("sqlite-doctor") do |dir|
      copy = File.join(dir, File.basename(database))
      SQLite3::Database.new(database, readonly: true) do |db|
        db.execute("VACUUM INTO ?", [copy])
      end

      begin
        ActiveRecord::Base.establish_connection(config.merge(database: copy))
        copy_pool = ActiveRecord::Base.connection_pool
        connection = base_connection
        unless main_database_path(connection) == File.realpath(copy)
          raise "refusing to migrate: the connection is not attached to the copy"
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
        watcher = SQLite3::Database.new(database, readonly: true)
        version = watcher.get_first_value("PRAGMA data_version")
        statements, failures, error = run_migrations(migrations_paths, copy_pool)
        rows_after = row_counts(base_connection)
        if watcher.get_first_value("PRAGMA data_version") != version
          raise "#{database} changed while migrating the copy: a migration, model or " \
                "other process committed to it (or ran a TRUNCATE checkpoint)"
        end
      ensure
        watcher&.close
        restore_connection(previous_config)
      end

      shrunk = rows_before.filter_map do |table, before|
        after = rows_after[table]
        "#{table} #{before} -> #{after}" if after && after < before
      end
      write_atomically(output, render(schema, foreign_keys, statements, failures, shrunk, error))
      { output: output, statements: statements.size - failures.size, failed: failures.size,
        shrunk_tables: shrunk, error: error }
    end
  end

  def run_migrations(migrations_paths, copy_pool)
    statements = []
    failures = []
    subscription = ActiveSupport::Notifications.subscribe("sql.active_record") do |*, payload|
      next if payload[:cached]
      # SQL a migration sends to another database is not part of this trace.
      next unless payload[:connection]&.pool.equal?(copy_pool)

      # A failed statement did not change the schema, so it must not count as
      # restoring an object. Keep it visible as a comment.
      failures << payload[:sql] if payload[:exception]
      statements << [payload[:sql], !payload[:exception].nil?]
    end
    begin
      ActiveRecord::MigrationContext.new(Array(migrations_paths)).migrate
      [statements, failures, nil]
    rescue StandardError => e
      [statements, failures, e]
    ensure
      ActiveSupport::Notifications.unsubscribe(subscription)
    end
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

      raise "refusing to migrate: connection pool #{pool.db_config.name} also uses #{database}"
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
        raise "refusing to migrate: cannot tell which file connection pool " \
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

  def render(schema, foreign_keys, statements, failures, shrunk, error)
    lines = []
    lines << "#{CAPTURE_HEADER} ActiveRecord #{ActiveRecord::VERSION::STRING}, " \
             "SQLite #{SQLite3::SQLITE_VERSION}"
    lines << "-- Migration target: temporary database copy."
    lines << "-- Migration failed: #{error.class}: #{error.message.lines.first&.strip}" if error
    lines << "-- #{failures.size} statement(s) failed and are kept below as comments." if failures.any?
    shrunk.each { |line| lines << "-- Observed on the copy: rows in #{line}" }
    lines << ""
    lines << "-- Baseline schema"
    schema.each { |sql| lines << terminate(sql) }
    lines << ""
    lines << "-- Connection state before migrating"
    lines << "PRAGMA foreign_keys = #{foreign_keys};"
    lines << ""
    lines << "-- Executed statements"
    statements.each do |sql, failed|
      lines << (failed ? "-- failed: #{sql.gsub(/\s+/, ' ')}" : terminate(sql))
    end
    lines.join("\n") + "\n"
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

  result = SqliteDoctorCapture.capture(
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

# Runs pending ActiveRecord migrations against a copy of a SQLite database and
# writes one SQL file for `sqlite-doctor`: the baseline schema, the
# connection's PRAGMA foreign_keys state, and every statement the migrations
# executed, including transaction boundaries.
#
# ActiveRecord::Base is pointed at the copy for the run and restored
# afterwards. A migration or model that opens its own connection to the
# original database is not redirected; the capture fails if the original
# changed while migrating.
#
# Standalone:
#   ruby sqlite_doctor_capture.rb DATABASE MIGRATIONS_DIR OUTPUT
# In a Rails app, see sqlite_doctor.rake.

require "active_record"
require "digest"
require "sqlite3"
require "tmpdir"

module SqliteDoctorCapture
  SIDE_FILES = ["", "-wal", "-shm", "-journal"].freeze

  module_function

  def capture(config:, migrations_paths:, output:)
    database = File.expand_path(config.fetch(:database).to_s)
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
        connection = base_connection
        unless main_database_path(connection) == File.realpath(copy)
          raise "refusing to migrate: the connection is not attached to the copy"
        end

        schema = connection.select_values(<<~SQL)
          SELECT sql FROM sqlite_schema
          WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
          ORDER BY rowid
        SQL
        foreign_keys = connection.select_value("PRAGMA foreign_keys")
        rows_before = row_counts(connection)
        fingerprint = fingerprint(database)

        statements, failures, error = run_migrations(migrations_paths)
        rows_after = row_counts(base_connection)
        if fingerprint(database) != fingerprint
          raise "#{database} changed while migrating the copy: a migration or model " \
                "opened its own connection to it"
        end
      ensure
        ActiveRecord::Base.connection_pool.disconnect!
        ActiveRecord::Base.establish_connection(previous_config) if previous_config
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

  def run_migrations(migrations_paths)
    statements = []
    failures = []
    subscription = ActiveSupport::Notifications.subscribe("sql.active_record") do |*, payload|
      next if payload[:cached]

      # A failed statement did not change the schema, so it must not count as
      # restoring an object. Keep it visible as a comment.
      if payload[:exception]
        failures << payload[:sql]
        statements << "-- failed: #{payload[:sql].gsub(/\s+/, ' ')}"
      else
        statements << payload[:sql]
      end
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

  def reject_output_aliasing(database, output)
    SIDE_FILES.each do |suffix|
      protected_path = database + suffix
      next unless output == protected_path ||
                  (File.exist?(output) && File.exist?(protected_path) &&
                   File.identical?(output, protected_path))

      raise ArgumentError, "refusing to write the capture over #{protected_path}"
    end
  end

  def reject_other_pools(database)
    base_pool = current_db_config && ActiveRecord::Base.connection_pool
    ActiveRecord::Base.connection_handler.connection_pool_list.each do |pool|
      next if pool.equal?(base_pool)

      path = pool.db_config.database.to_s
      next if path.empty? || !File.exist?(File.expand_path(path))
      next unless File.identical?(File.expand_path(path), database)

      raise "refusing to migrate: connection pool #{pool.db_config.name} also uses #{database}"
    end
  end

  def fingerprint(database)
    SIDE_FILES.map do |suffix|
      path = database + suffix
      File.file?(path) ? Digest::SHA256.file(path).hexdigest : nil
    end
  end

  def row_counts(connection)
    tables = connection.select_values(<<~SQL)
      SELECT name FROM sqlite_schema
      WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
    SQL
    tables.to_h do |table|
      [table, connection.select_value("SELECT count(*) FROM #{connection.quote_table_name(table)}")]
    end
  end

  def render(schema, foreign_keys, statements, failures, shrunk, error)
    lines = []
    lines << "-- sqlite-doctor capture: ActiveRecord #{ActiveRecord::VERSION::STRING}, " \
             "SQLite #{SQLite3::SQLITE_VERSION}"
    lines << "-- Migrations ran on a copy of the database; the original was not changed."
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
    statements.each { |sql| lines << (sql.start_with?("-- failed: ") ? sql : terminate(sql)) }
    lines.join("\n") + "\n"
  end

  def terminate(sql)
    sql.sub(/;?\s*\z/, ";")
  end

  def write_atomically(output, content)
    temporary = "#{output}.#{Process.pid}.tmp"
    File.write(temporary, content)
    File.rename(temporary, output)
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

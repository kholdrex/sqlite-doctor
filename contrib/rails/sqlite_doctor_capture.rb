# Runs pending ActiveRecord migrations against a copy of a SQLite database and
# writes one SQL file for `sqlite-doctor`: the baseline schema, the
# connection's PRAGMA foreign_keys state, and every statement the migrations
# executed, including transaction boundaries. The original database is never
# migrated.
#
# Standalone:
#   ruby sqlite_doctor_capture.rb DATABASE MIGRATIONS_DIR OUTPUT
# In a Rails app, see sqlite_doctor.rake.

require "active_record"
require "sqlite3"
require "tmpdir"

module SqliteDoctorCapture
  module_function

  def capture(config:, migrations_paths:, output:)
    database = File.expand_path(config.fetch(:database).to_s)
    raise ArgumentError, "#{database} does not exist" unless File.file?(database)

    Dir.mktmpdir("sqlite-doctor") do |dir|
      copy = File.join(dir, File.basename(database))
      SQLite3::Database.new(database, readonly: true) do |db|
        db.execute("VACUUM INTO ?", [copy])
      end

      ActiveRecord::Base.establish_connection(config.merge(database: copy))
      connection = ActiveRecord::Base.lease_connection
      attached = connection.select_rows("PRAGMA database_list").find { |row| row[1] == "main" }
      unless attached && File.realpath(attached[2]) == File.realpath(copy)
        raise "refusing to migrate: connection is not the copy (#{attached.inspect})"
      end

      schema = connection.select_values(<<~SQL)
        SELECT sql FROM sqlite_schema
        WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
        ORDER BY rowid
      SQL
      foreign_keys = connection.select_value("PRAGMA foreign_keys")
      rows_before = row_counts(connection)

      statements = []
      subscription = ActiveSupport::Notifications.subscribe("sql.active_record") do |*, payload|
        statements << payload[:sql] unless payload[:cached]
      end
      error = nil
      begin
        ActiveRecord::MigrationContext.new(Array(migrations_paths)).migrate
      rescue StandardError => e
        error = e
      ensure
        ActiveSupport::Notifications.unsubscribe(subscription)
      end
      rows_after = row_counts(connection)
      ActiveRecord::Base.connection_pool.disconnect!

      shrunk = rows_before.filter_map do |table, before|
        after = rows_after[table]
        "#{table} #{before} -> #{after}" if after && after < before
      end
      File.write(output, render(schema, foreign_keys, statements, shrunk, error))
      { output: output, statements: statements.size, shrunk_tables: shrunk, error: error }
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

  def render(schema, foreign_keys, statements, shrunk, error)
    lines = []
    lines << "-- sqlite-doctor capture: ActiveRecord #{ActiveRecord::VERSION::STRING}, " \
             "SQLite #{SQLite3::SQLITE_VERSION}"
    lines << "-- Migrations ran on a copy of the database; the original was not changed."
    lines << "-- Migration failed: #{error.class}: #{error.message.lines.first&.strip}" if error
    shrunk.each { |line| lines << "-- Observed on the copy: rows in #{line}" }
    lines << ""
    lines << "-- Baseline schema"
    schema.each { |sql| lines << terminate(sql) }
    lines << ""
    lines << "-- Connection state before migrating"
    lines << "PRAGMA foreign_keys = #{foreign_keys};"
    lines << ""
    lines << "-- Executed statements"
    statements.each { |sql| lines << terminate(sql) }
    lines.join("\n") + "\n"
  end

  def terminate(sql)
    sql.sub(/;?\s*\z/, ";")
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

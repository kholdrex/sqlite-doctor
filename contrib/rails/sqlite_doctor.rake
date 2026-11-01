# Copy this file and sqlite_doctor_capture.rb into lib/tasks/, then run:
#   bin/rails sqlite_doctor:capture OUTPUT=tmp/migration.sql
#   sqlite-doctor tmp/migration.sql
# Only the primary database is captured.

namespace :sqlite_doctor do
  desc "Run pending migrations on a copy of the SQLite database and write the SQL for sqlite-doctor"
  task capture: :environment do
    require_relative "sqlite_doctor_capture"

    db_config = ActiveRecord::Base.connection_db_config
    abort "sqlite_doctor:capture supports the sqlite3 adapter only" unless db_config.adapter == "sqlite3"

    output = ENV.fetch("OUTPUT", "tmp/sqlite-doctor-capture.sql")
    result = SqliteDoctorCapture.capture(
      config: db_config.configuration_hash,
      migrations_paths: db_config.migrations_paths || ActiveRecord::Migrator.migrations_paths,
      output: output
    )
    puts "Wrote #{result[:statements]} statements to #{output}"
    result[:shrunk_tables].each { |line| puts "Rows lost on the copy: #{line}" }
    abort "Migration failed on the copy: #{result[:error].message}" if result[:error]
  end
end

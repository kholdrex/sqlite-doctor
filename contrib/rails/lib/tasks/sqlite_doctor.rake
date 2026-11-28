# bin/rails sqlite_doctor:check [BASE=<git revision>] [DATABASE=<name>] [OUTPUT=tmp/migration.sql]
#   Runs the pending migrations on a copy of the SQLite database, or with BASE
#   on a database built from the schema at that revision, and scans the SQL
#   with the sqlite-doctor binary. Exits 1 with findings and 2 when the check
#   is incomplete or fails; on success it returns, so later tasks still run.
# bin/rails sqlite_doctor:capture [DATABASE=<name>] [OUTPUT=tmp/migration.sql]
#   Only writes the captured SQL.

module SqliteDoctor
  module Tasks
    module_function

    # The database to check: DATABASE, or the only one in this environment
    # whose migrations Rails runs.
    def db_config
      configs = ActiveRecord::Base.configurations.configs_for(env_name: Rails.env).reject(&:replica?)
      configs = configs.select(&:database_tasks?) if configs.first.respond_to?(:database_tasks?)
      name = ENV["DATABASE"].to_s.strip
      config =
        if !name.empty?
          configs.find { |candidate| candidate.name == name } ||
            raise(SqliteDoctor::Check::Error, "no database named #{name} in #{Rails.env}")
        elsif configs.empty?
          raise SqliteDoctor::Check::Error, "no database to migrate is configured for #{Rails.env}"
        elsif configs.size == 1
          configs.first
        else
          raise SqliteDoctor::Check::Error,
                "#{Rails.env} migrates #{configs.size} databases (#{configs.map(&:name).join(', ')}); " \
                "choose one with DATABASE=<name>"
        end
      unless config.adapter == "sqlite3"
        raise SqliteDoctor::Check::Error, "#{config.name} uses the #{config.adapter} adapter; only sqlite3 is supported"
      end

      config
    end

    def migrations_paths(config)
      config.migrations_paths || ActiveRecord::Migrator.migrations_paths
    end

    def schema_format(config)
      config.respond_to?(:schema_format) ? config.schema_format : ActiveRecord.schema_format
    end

    # The database file, resolved like Rails: a relative path against
    # Rails.root. Which file a `file:` URI opens depends on the sqlite3 gem,
    # so those are refused rather than guessed.
    def database_file(config)
      if config.database.to_s.start_with?("file:")
        raise SqliteDoctor::Check::Error, "#{config.name} is a file: URI; set a plain path, or use BASE"
      end

      SqliteDoctor::Capture.pool_database_path(config) ||
        raise(SqliteDoctor::Check::Error, "#{config.name} is not a file-backed database")
    end

    def fail_with(error)
      warn "sqlite-doctor: #{error.message}"
      exit 2
    end
  end
end

namespace :sqlite_doctor do
  desc "Check pending migrations for SQLite rebuilds that lose rows (BASE=git revision, DATABASE=name, OUTPUT=file)"
  task check: "db:load_config" do
    status =
      begin
        config = SqliteDoctor::Tasks.db_config
        base = ENV["BASE"].to_s.strip
        base = nil if base.empty?
        format = SqliteDoctor::Tasks.schema_format(config)
        settings = config.configuration_hash
        settings = settings.merge(database: SqliteDoctor::Tasks.database_file(config)) unless base
        SqliteDoctor::Check.run(
          config: settings,
          migrations_paths: SqliteDoctor::Tasks.migrations_paths(config),
          base: base,
          schema_file: ActiveRecord::Tasks::DatabaseTasks.schema_dump_path(config, format),
          schema_format: format,
          root: Rails.root.to_s,
          output: ENV["OUTPUT"],
          name: config.name,
          env_name: config.env_name
        )
      rescue SqliteDoctor::Check::Error, SqliteDoctor::Capture::Refused, ArgumentError => e
        SqliteDoctor::Tasks.fail_with(e)
      end
    exit status unless status.zero?
  end

  desc "Run pending migrations on a copy of the SQLite database and write the SQL for sqlite-doctor"
  task capture: "db:load_config" do
    config = SqliteDoctor::Tasks.db_config
    output = ENV.fetch("OUTPUT", "tmp/sqlite-doctor-capture.sql")
    result = SqliteDoctor::Capture.capture(
      config: config.configuration_hash.merge(database: SqliteDoctor::Tasks.database_file(config)),
      migrations_paths: SqliteDoctor::Tasks.migrations_paths(config),
      output: output,
      root: Rails.root.to_s,
      name: config.name,
      env_name: config.env_name
    )
    puts "Wrote #{result[:statements]} statements to #{output}"
    result[:shrunk_tables].each { |line| puts "Rows lost on the copy: #{line}" }
    if result[:error]
      SqliteDoctor::Tasks.fail_with(SqliteDoctor::Check::Error.new("the migration raised #{result[:error].class}; " \
                                                                   "its message is in #{output}"))
    end
  rescue SqliteDoctor::Check::Error, SqliteDoctor::Capture::Refused, ArgumentError => e
    SqliteDoctor::Tasks.fail_with(e)
  end
end

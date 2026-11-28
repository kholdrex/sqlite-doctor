# Captures the pending migrations, scans the capture with the sqlite-doctor
# binary and prints its findings. Returns the binary's exit status: 0 nothing
# found, 1 findings, 2 an incomplete check or an error.

require "json"
require "open3"
require "pathname"
require "tmpdir"
require_relative "capture"

module SqliteDoctor::Check
  class Error < StandardError; end

  # Rails' migration file name pattern.
  MIGRATION_FILE = /\A([0-9]+)_([_a-z0-9]*)\.?([_a-z0-9]*)?\.rb\z/

  module_function

  # base: a git revision. The database is then built from the schema file and
  # the migration files at that revision instead of the configured database.
  def run(config:, migrations_paths:, base: nil, schema_file: nil, schema_format: :ruby,
          root: Dir.pwd, output: nil, binary: nil, io: $stdout, name: nil, env_name: nil)
    binary = find_binary(binary)
    SqliteDoctor::Capture.reject_in_memory_connection
    Dir.mktmpdir("sqlite-doctor-check") do |dir|
      watch = nil
      if base
        watch = protect_configured_database(config)
        config = config.merge(
          database: database_at(base, dir: dir, root: root, schema_file: schema_file,
                                      schema_format: schema_format, config: config,
                                      migrations_paths: migrations_paths, name: name, env_name: env_name)
        )
        io.puts "Checking against the schema at #{base} (#{File.basename(schema_file)}, no rows). Findings come " \
                "from the SQL alone: a migration whose SQL depends on rows, or a trigger or view the schema file " \
                "leaves out, is not covered."
      end
      output ||= File.join(dir, "capture.sql")
      result = SqliteDoctor::Capture.capture(config: config, migrations_paths: migrations_paths, output: output,
                                             root: root, name: name, env_name: env_name, watch: watch)
      report(binary, result, io)
    end
  rescue StandardError, ScriptError => e
    known = e.is_a?(Error) || e.is_a?(SqliteDoctor::Capture::Refused)
    io.puts(known ? "sqlite-doctor: #{e.message}" : "sqlite-doctor: #{e.class}: #{e.message}")
    2
  end

  def find_binary(binary)
    binary ||= ENV.fetch("SQLITE_DOCTOR", nil)
    return binary if binary && File.file?(binary) && File.executable?(binary)
    raise Error, "SQLITE_DOCTOR=#{binary} is not an executable file" if binary

    found = ENV.fetch("PATH", "").split(File::PATH_SEPARATOR).map { |dir| File.join(dir, "sqlite-doctor") }
                .find { |path| File.file?(path) && File.executable?(path) }
    found || raise(Error, "the sqlite-doctor binary is not on PATH; install it with `cargo install sqlite-doctor`, " \
                          "the kholdrex/sqlite-doctor GitHub Action or a release archive, or set SQLITE_DOCTOR")
  end

  def report(binary, result, io)
    stdout, stderr, status = Open3.capture3(binary, "--format", "json", result[:output])
    report = begin
      JSON.parse(stdout)
    rescue JSON::ParserError
      nil
    end
    unless report.is_a?(Hash) && report.key?("complete")
      detail = stderr.strip.empty? ? "" : ": #{stderr.strip}"
      raise Error, "#{binary} did not produce sqlite-doctor 0.3.0 JSON output; 0.3.0 or newer is needed#{detail}"
    end

    pending = result[:pending]
    if pending.nil?
      io.puts "The pending migrations could not be determined."
    elsif pending.empty?
      io.puts "No pending migrations."
    else
      io.puts "#{pending.size} pending migration(s):"
      pending.each { |name| io.puts "  #{name}" }
    end
    report["findings"].each do |finding|
      io.puts
      io.puts "#{finding['code']}: #{finding['title']}"
      io.puts "  --> #{location(finding['location'])} (#{report['source']}:#{finding['line']})"
      io.puts "  #{finding['detail']}"
      io.puts "  help: #{finding['remediation']}"
    end
    result[:shrunk_tables].each { |line| io.puts "Rows lost on the copy: #{line}" }
    unless report["complete"]
      io.puts
      io.puts "Check incomplete:"
      report["incomplete"].each { |item| io.puts "- #{item['reason']}" }
    end
    if result[:error]
      io.puts "Migration failed on the copy: #{result[:error].class}. Its message can quote SQL with data, so it " \
              "is only in the capture: rerun with OUTPUT=<file> to keep it."
    end

    status = status.exitstatus
    return 2 if status.nil? || status > 1 || !report["complete"]

    status
  end

  def location(location)
    return location["source"] unless location["line"]

    "#{location['source']}:#{location['line']}"
  end

  # With BASE the configured database is replaced by a temporary one, so a
  # model connected to it on its own pool would write to it for real. Returns
  # the file to watch while migrating, if it exists.
  def protect_configured_database(config)
    db_config = ActiveRecord::DatabaseConfigurations::HashConfig.new("check", "primary", config)
    path = SqliteDoctor::Capture.pool_database_path(db_config)
    return unless path && File.exist?(path)

    path = File.realpath(path)
    SqliteDoctor::Capture.reject_other_pools(path)
    path
  end

  # A SQLite database with the schema at `base` and the migrations of `base`
  # recorded as applied, so the pending migrations are the ones added since.
  def database_at(base, dir:, root:, schema_file:, schema_format:, config:, migrations_paths:, name: nil,
                  env_name: nil)
    raise Error, "BASE must name a git revision" if base.empty? || base.start_with?("-") || base.match?(/[[:cntrl:]]/)
    raise Error, "no schema file is configured" unless schema_file

    top = git(root, "rev-parse", "--show-toplevel").chomp
    prefix = git(root, "rev-parse", "--show-prefix").chomp
    commit = git(top, "rev-parse", "--verify", "--end-of-options", "#{base}^{commit}").chomp
    application = File.expand_path(root)
    # Relative to the repository: the application's prefix from git, then the
    # path from the application, which may lead to a sibling directory.
    in_repository = lambda do |path|
      from_application = Pathname(File.expand_path(path, application)).relative_path_from(Pathname(application))
      relative = Pathname(prefix).join(from_application).cleanpath.to_s
      raise Error, "#{path} is outside the git repository" if relative == ".." || relative.start_with?("../")

      relative
    end

    schema_path = in_repository.call(schema_file)
    entry = git(top, "ls-tree", "-z", commit, "--", schema_path).split("\0").first
    raise Error, "#{schema_path} does not exist at #{base}" unless entry

    mode, type, object = entry.split("\t", 2).first.split(" ")
    if mode == "120000"
      raise Error, "#{schema_path} is a symbolic link at #{base}; point the schema setting at the file itself"
    end
    raise Error, "#{schema_path} is not a file at #{base}" unless type == "blob"

    schema = git(top, "cat-file", "blob", object)
    # Rails finds migrations with Dir["#{path}/**/[0-9]*_*.rb"], which skips
    # hidden files and directories.
    versions = Array(migrations_paths).flat_map do |path|
      raise Error, "migrations path #{path} is a glob pattern, which BASE does not support" if path.to_s.match?(/[*?\[{]/)

      directory = in_repository.call(path)
      git(top, "ls-tree", "-r", "-z", "--name-only", commit, "--", "#{directory}/")
        .split("\0").filter_map do |entry|
          parts = entry.delete_prefix("#{directory}/").split("/")
          next if parts.any? { |part| part.start_with?(".") }

          parts.last[MIGRATION_FILE, 1]
        end
    end

    database = File.join(dir, "base.sqlite3")
    if schema_format.to_sym == :sql
      SQLite3::Database.new(database) { |db| load_structure(db, schema) }
    else
      load_ruby_schema(schema, database, dir, config, migrations_paths, name, env_name)
    end
    reset_ledger(database, versions)
    database
  end

  # The table Rails reads applied migrations from, with any prefix or suffix.
  def reset_ledger(database, versions)
    ledger = "#{ActiveRecord::Base.table_name_prefix}#{ActiveRecord::Base.schema_migrations_table_name}" \
             "#{ActiveRecord::Base.table_name_suffix}"
    quoted = %("#{ledger.gsub('"', '""')}")
    SQLite3::Database.new(database) do |db|
      db.execute("CREATE TABLE IF NOT EXISTS #{quoted} (version varchar NOT NULL PRIMARY KEY)")
      db.execute("DELETE FROM #{quoted}")
      versions.uniq.each { |version| db.execute("INSERT INTO #{quoted} (version) VALUES (?)", [version]) }
    end
  end

  # schema.rb marks migrations up to its version as applied; reset_ledger
  # replaces that list with the migrations at the base revision.
  # A table definition at the start of the remaining dump, with no quoted
  # text inside, as the sqlite3 shell writes the tables SQLite creates itself.
  TABLE_DEFINITION = /\A\s*CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?["`\[']?(\w+)["`\]']?\s*\([^;'"`]*\)\s*;/i
  ONLY_COMMENTS = %r{\A(?:\s|--[^\n]*(?:\n|\z)|/\*.*?\*/)*\z}m

  # Runs a structure dump one statement at a time, with SQLite finding each
  # statement's end. Skips tables SQLite creates itself: reserved sqlite_*
  # tables and the shadow tables of a virtual table already created. SQLite
  # rejects both while preparing them.
  def load_structure(db, schema)
    rest = schema
    until rest.match?(ONLY_COMMENTS)
      begin
        statement = db.prepare(rest)
      rescue SQLite3::SQLException => e
        table = rest.match(TABLE_DEFINITION)
        raise unless table && created_by_sqlite?(db, table[1], e.message)

        rest = table.post_match
        next
      end
      begin
        rest = statement.remainder
        statement.execute!
      ensure
        statement.close
      end
    end
  end

  def created_by_sqlite?(db, table, message)
    return table.downcase.start_with?("sqlite_") if message.include?("reserved for internal use")
    return false unless message.include?("already exists")

    db.execute("SELECT name FROM sqlite_schema WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL TABLE%'")
      .flatten.any? { |virtual| table.start_with?("#{virtual}_") }
  end

  def load_ruby_schema(schema, database, dir, config, migrations_paths, name, env_name)
    file = File.join(dir, "schema.rb")
    File.write(file, schema)
    previous = SqliteDoctor::Capture.current_db_config
    begin
      ActiveRecord::Base.establish_connection(
        SqliteDoctor::Capture.named_config(config.merge(database: database, migrations_paths: migrations_paths),
                                           name, env_name)
      )
      ActiveRecord::Migration.suppress_messages { load(file) }
    ensure
      SqliteDoctor::Capture.restore_connection(previous)
    end
  end

  # Paths are passed as literal pathspecs whatever the environment says.
  GIT_ENV = { "GIT_LITERAL_PATHSPECS" => "1", "GIT_GLOB_PATHSPECS" => nil, "GIT_NOGLOB_PATHSPECS" => nil,
              "GIT_ICASE_PATHSPECS" => nil }.freeze

  def git(directory, *arguments)
    stdout, stderr, status = Open3.capture3(GIT_ENV, "git", "-C", directory.to_s, *arguments, binmode: true)
    raise Error, "git #{arguments.first} failed: #{stderr.strip}" unless status.success?

    stdout.force_encoding(Encoding::UTF_8)
  end
end

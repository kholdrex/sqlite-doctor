require "bundler/inline"

ar_version = ENV.fetch("AR_VERSION", "~> 8.1")
gemfile(ENV["CI"] == "true") do
  source "https://rubygems.org"
  gem "activerecord", ar_version
  gem "sqlite3", ar_version.start_with?("7.1") ? "~> 1.4" : ">= 2.1"
  gem "minitest", "~> 5.0"
end

require "minitest/autorun"
require "stringio"
require "tmpdir"
require_relative "../lib/sqlite_doctor"

ActiveRecord::Migration.verbose = false

class SecondaryRecord < ActiveRecord::Base
  self.abstract_class = true
end

# Runs against the sqlite-doctor binary in SQLITE_DOCTOR, or the debug build
# of this repository.
class CheckTest < Minitest::Test
  BINARY = ENV.fetch("SQLITE_DOCTOR") { File.expand_path("../../../target/debug/sqlite-doctor", __dir__) }

  SCHEMA = <<~SQL.freeze
    CREATE TABLE tenants (id INTEGER PRIMARY KEY);
    CREATE TABLE parents (id INTEGER PRIMARY KEY, tenant_id INTEGER, name TEXT);
    CREATE TABLE children (id INTEGER PRIMARY KEY,
      parent_id INTEGER NOT NULL REFERENCES parents(id) ON DELETE CASCADE);
    INSERT INTO tenants VALUES (1);
    INSERT INTO parents VALUES (1, 1, 'a');
    INSERT INTO children VALUES (1, 1);
  SQL

  REBUILD = "add_foreign_key :parents, :tenants, column: :tenant_id".freeze

  def setup
    skip "build sqlite-doctor or set SQLITE_DOCTOR" unless File.executable?(BINARY)
    @dir = Dir.mktmpdir("check-test")
    @database = File.join(@dir, "app.sqlite3")
    SQLite3::Database.new(@database) { |db| db.execute_batch(SCHEMA) }
    @migrations = File.join(@dir, "db", "migrate")
    FileUtils.mkdir_p(@migrations)
    @io = StringIO.new
  end

  def teardown
    ActiveRecord::Base.connection_handler.clear_all_connections!
    FileUtils.remove_entry(@dir) if @dir
  end

  def migration(version, body, name: "change#{version}")
    File.write(File.join(@migrations, "#{version}_#{name}.rb"), <<~RUBY)
      class #{name.split('_').map(&:capitalize).join} < ActiveRecord::Migration[7.1]
        def change
          #{body}
        end
      end
    RUBY
  end

  def check(migrations_paths: @migrations, **options)
    SqliteDoctor::Check.run(
      config: { adapter: "sqlite3", database: @database },
      migrations_paths: migrations_paths, root: @dir, binary: BINARY, io: @io, **options
    )
  end

  def test_a_rebuild_that_cascades_is_reported_with_its_migration
    migration(20260101000001, REBUILD)

    assert_equal 1, check
    assert_includes @io.string, "1 pending migration(s):\n  db/migrate/20260101000001_change20260101000001.rb"
    assert_match(%r{SQLD001: .*\n  --> db/migrate/20260101000001_change20260101000001\.rb\n}, @io.string)
    refute_includes @io.string, "capture.sql"
  end

  def test_a_kept_capture_is_named_beside_each_finding
    migration(20260101000001, REBUILD)
    kept = File.join(@dir, "kept.sql")

    assert_equal 1, check(output: kept)
    assert_match(%r{SQLD001: .*\n  --> db/migrate/20260101000001_change20260101000001\.rb \(#{Regexp.escape(kept)}:\d+\)\n}, @io.string)
  end

  def test_no_pending_migrations_is_a_clean_complete_check
    assert_equal 0, check
    assert_includes @io.string, "No pending migrations."
  end

  def test_a_failed_migration_is_an_incomplete_check
    migration(20260101000001, %(execute "INSERT INTO parents VALUES (1, 1, 'duplicate')"))

    assert_equal 2, check
    assert_includes @io.string, "Check incomplete:"
    assert_includes @io.string, "Migration failed on the copy"
  end

  def test_a_missing_or_old_binary_fails_the_check
    status = SqliteDoctor::Check.run(config: { adapter: "sqlite3", database: @database },
                                     migrations_paths: @migrations, binary: File.join(@dir, "missing"), io: @io)
    assert_equal 2, status
    assert_includes @io.string, "is not an executable file"

    old = File.join(@dir, "old-sqlite-doctor")
    File.write(old, %(#!/bin/sh\necho '{"version": 1, "source": "x", "findings": []}'\n))
    File.chmod(0o755, old)
    assert_equal 2, check(binary: old)
    assert_includes @io.string, "0.3.0 or newer is needed"
  end

  def test_base_builds_the_database_from_the_schema_and_migrations_at_that_revision
    File.delete(@database)
    migration(20260101000001, <<~RUBY, name: "create_tables")
      create_table :parents
      create_table :children do |t|
        t.references :parent, null: false, foreign_key: { on_delete: :cascade }
      end
    RUBY
    File.write(File.join(@dir, "db", "schema.rb"), <<~RUBY)
      ActiveRecord::Schema[7.1].define(version: 2026_01_01_000001) do
        create_table "parents" do |t|
          t.integer "tenant_id"
        end
        create_table "children" do |t|
          t.integer "parent_id", null: false
        end
        create_table "tenants"
        add_foreign_key "children", "parents", on_delete: :cascade
      end
    RUBY
    git("init", "-q")
    git("add", ".")
    git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "base")
    migration(20250101000002, REBUILD, name: "older_timestamp")

    status = check(base: "HEAD", schema_file: File.join(@dir, "db", "schema.rb"), schema_format: :ruby)

    assert_equal 1, status, @io.string
    assert_includes @io.string, "1 pending migration(s):\n  db/migrate/20250101000002_older_timestamp.rb\n"
    assert_includes @io.string, "SQLD001"
    refute File.exist?(@database)
  end

  def base_repository(schema_name: "schema.rb", schema: nil)
    File.delete(@database)
    migration(20260101000001, "create_table :parents", name: "create_parents")
    File.write(File.join(@dir, "db", schema_name), schema || <<~RUBY)
      ActiveRecord::Schema[7.1].define(version: 2026_01_02_000000) do
        create_table "tenants"
        create_table "parents" do |t|
          t.integer "tenant_id"
        end
        create_table "children" do |t|
          t.integer "parent_id", null: false
        end
        add_foreign_key "children", "parents", on_delete: :cascade
      end
    RUBY
    git("init", "-q")
    git("add", ".")
    git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "base")
  end

  def test_base_ignores_pathspec_settings_in_the_environment
    base_repository
    migration(20260101000002, REBUILD, name: "after_base")
    saved = ENV.fetch("GIT_LITERAL_PATHSPECS", nil)
    ENV["GIT_LITERAL_PATHSPECS"] = "1"

    status = check(base: "HEAD", schema_file: File.join(@dir, "db", "schema.rb"))

    assert_equal 1, status, @io.string
    assert_includes @io.string, "1 pending migration(s):\n  db/migrate/20260101000002_after_base.rb\n"
  ensure
    ENV["GIT_LITERAL_PATHSPECS"] = saved
  end

  def test_base_uses_the_ledger_rails_reads
    base_repository
    migration(20260101000002, REBUILD, name: "older_than_schema")
    ActiveRecord::Base.schema_migrations_table_name = "release_versions"

    status = check(base: "HEAD", schema_file: File.join(@dir, "db", "schema.rb"))

    assert_equal 1, status, @io.string
    assert_includes @io.string, "db/migrate/20260101000002_older_than_schema.rb"
  ensure
    ActiveRecord::Base.schema_migrations_table_name = "schema_migrations"
  end

  def test_base_refuses_a_second_pool_on_the_configured_database
    base_repository
    SQLite3::Database.new(@database) { |db| db.execute_batch(SCHEMA) }
    SecondaryRecord.establish_connection(adapter: "sqlite3", database: @database)
    migration(20260102000001, REBUILD)

    assert_equal 2, check(base: "HEAD", schema_file: File.join(@dir, "db", "schema.rb"))
    assert_includes @io.string, "also uses"
  ensure
    SecondaryRecord.remove_connection
  end

  def test_an_in_memory_connection_is_refused
    ActiveRecord::Base.establish_connection(adapter: "sqlite3", database: ":memory:")
    ActiveRecord::Base.connection.execute("CREATE TABLE kept (id INTEGER)")

    assert_equal 2, check
    assert_includes @io.string, "in-memory database"
    assert_equal ["kept"], ActiveRecord::Base.connection.select_values("SELECT name FROM sqlite_schema")
  ensure
    ActiveRecord::Base.remove_connection
  end

  def test_base_refuses_a_symlinked_schema
    base_repository(schema_name: "schema_v2.rb")
    File.symlink("schema_v2.rb", File.join(@dir, "db", "schema.rb"))
    git("add", ".")
    git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "link")

    assert_equal 2, check(base: "HEAD", schema_file: File.join(@dir, "db", "schema.rb"))
    assert_includes @io.string, "is a symbolic link at HEAD"
  end

  def test_base_loads_a_structure_dump_with_sqlite_internal_tables
    base_repository(schema_name: "structure.sql", schema: <<~SQL)
      CREATE TABLE IF NOT EXISTS "tenants" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL);
      CREATE TABLE sqlite_sequence(name,seq);
      CREATE TABLE IF NOT EXISTS "parents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer);
      CREATE TABLE IF NOT EXISTS "children" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL,
        "parent_id" integer NOT NULL, FOREIGN KEY ("parent_id") REFERENCES "parents" ("id") ON DELETE CASCADE);
      CREATE TABLE IF NOT EXISTS "schema_migrations" ("version" varchar NOT NULL PRIMARY KEY);
      INSERT INTO "schema_migrations" (version) VALUES ('20260101000001');
    SQL
    migration(20260102000001, REBUILD)

    status = check(base: "HEAD", schema_file: File.join(@dir, "db", "structure.sql"), schema_format: :sql)

    assert_equal 1, status, @io.string
    assert_includes @io.string, "SQLD001"
  end

  def test_failed_discovery_does_not_claim_no_pending_migrations
    migration(20260101000001, "", name: "same")
    migration(20260101000002, "", name: "same")

    assert_equal 2, check
    assert_includes @io.string, "The pending migrations could not be determined."
    refute_includes @io.string, "No pending migrations."
  end

  def test_base_skips_hidden_migration_directories_and_refuses_glob_paths
    base_repository
    FileUtils.mkdir_p(File.join(@migrations, ".archive"))
    migration(20260101000002, REBUILD, name: "promoted")
    File.rename(File.join(@migrations, "20260101000002_promoted.rb"),
                File.join(@migrations, ".archive", "20260101000002_promoted.rb"))
    git("add", ".")
    git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "hidden")
    File.rename(File.join(@migrations, ".archive", "20260101000002_promoted.rb"),
                File.join(@migrations, "20260101000002_promoted.rb"))

    assert_equal 1, check(base: "HEAD", schema_file: File.join(@dir, "db", "schema.rb")), @io.string
    assert_includes @io.string, "db/migrate/20260101000002_promoted.rb"

    status = check(base: "HEAD", schema_file: File.join(@dir, "db", "schema.rb"),
                   migrations_paths: File.join(@dir, "db", "mig[r]ate"))
    assert_equal 2, status
    assert_includes @io.string, "is a glob pattern"
  end

  def test_structure_dump_keeps_string_literals_and_skips_shadow_tables
    template = "CREATE TABLE sqlite_sequence(name,seq);"
    base_repository(schema_name: "structure.sql", schema: <<~SQL)
      CREATE TABLE IF NOT EXISTS "tenants" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL);
      CREATE TABLE sqlite_sequence(name,seq);
      CREATE TABLE IF NOT EXISTS "parents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer);
      CREATE TABLE IF NOT EXISTS "children" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL,
        "parent_id" integer NOT NULL, FOREIGN KEY ("parent_id") REFERENCES "parents" ("id") ON DELETE CASCADE);
      CREATE VIEW "templates" AS SELECT '
      #{template}
      ' AS body;
      CREATE VIRTUAL TABLE search USING fts5(content);
      CREATE TABLE 'search_data'(id INTEGER PRIMARY KEY, block BLOB);
      CREATE TABLE IF NOT EXISTS "schema_migrations" ("version" varchar NOT NULL PRIMARY KEY);
    SQL
    migration(20260102000001, <<~RUBY)
      raise "the view lost its text" unless select_value("SELECT body FROM templates").include?(#{template.inspect})
      #{REBUILD}
    RUBY

    status = check(base: "HEAD", schema_file: File.join(@dir, "db", "structure.sql"), schema_format: :sql)

    assert_equal 1, status, @io.string
  end

  def test_an_in_memory_connection_opened_with_the_memory_flag_is_refused
    skip "sqlite3 gem has no MEMORY flag" unless defined?(SQLite3::Constants::Open::MEMORY)

    flags = SQLite3::Constants::Open::READWRITE | SQLite3::Constants::Open::CREATE | SQLite3::Constants::Open::MEMORY
    ActiveRecord::Base.establish_connection(adapter: "sqlite3", database: File.join(@dir, "named"), flags: flags)
    ActiveRecord::Base.connection.execute("CREATE TABLE kept (id INTEGER)")

    assert_equal 2, check
    assert_includes @io.string, "in-memory database"
    assert_equal ["kept"], ActiveRecord::Base.connection.select_values("SELECT name FROM sqlite_schema")
  ensure
    ActiveRecord::Base.remove_connection
  end

  def test_base_reads_migrations_from_a_sibling_directory_of_the_app
    File.delete(@database)
    app = File.join(@dir, "apps", "shop")
    engine = File.join(@dir, "engines", "billing", "db", "migrate")
    FileUtils.mkdir_p([File.join(app, "db"), engine])
    File.write(File.join(engine, "20260101000001_create_parents.rb"),
               "class CreateParents < ActiveRecord::Migration[7.1]; def change; create_table :parents; end; end")
    base_repository_at(app)
    File.write(File.join(engine, "20260102000001_link.rb"),
               "class Link < ActiveRecord::Migration[7.1]; def change; #{REBUILD}; end; end")

    status = SqliteDoctor::Check.run(
      config: { adapter: "sqlite3", database: File.join(app, "db", "development.sqlite3") },
      migrations_paths: engine, base: "HEAD", root: app,
      schema_file: File.join(app, "db", "schema.rb"), binary: BINARY, io: @io
    )

    assert_equal 1, status, @io.string
    assert_match(%r{1 pending migration\(s\):\n  \S*engines/billing/db/migrate/20260102000001_link\.rb\n}, @io.string)
  end

  def base_repository_at(app)
    File.write(File.join(app, "db", "schema.rb"), <<~RUBY)
      ActiveRecord::Schema[7.1].define(version: 2026_01_01_000001) do
        create_table "tenants"
        create_table "parents" do |t|
          t.integer "tenant_id"
        end
        create_table "children" do |t|
          t.integer "parent_id", null: false
        end
        add_foreign_key "children", "parents", on_delete: :cascade
      end
    RUBY
    git("init", "-q")
    git("add", ".")
    git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "base")
  end

  def test_a_failed_migration_does_not_print_its_sql
    migration(20260101000001, %(execute "INSERT INTO missing_table VALUES ('customer-literal-probe')"))

    assert_equal 2, check
    refute_includes @io.string, "customer-literal-probe"
    assert_includes @io.string, "the migration raised "
  end

  def test_base_rejects_an_option_like_revision
    assert_equal 2, check(base: "--output=x", schema_file: "db/schema.rb")
    assert_includes @io.string, "BASE must name a git revision"
  end

  def git(*arguments)
    system("git", "-C", @dir, *arguments, exception: true)
  end
end

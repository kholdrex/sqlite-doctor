require "minitest/autorun"
require "open3"
require "fileutils"
require "tmpdir"

# Boots a minimal Rails application in a child process, loads the gem through
# its gemspec and Railtie and runs `sqlite_doctor:check` through Rake, as
# `bin/rails` does. The application lives in a subdirectory of its git
# repository. Installed gems are reused; with CI=true or INSTALL=1 the child
# installs what is missing, and Bundler may also fetch a missing gem otherwise.
class RailtieTest < Minitest::Test
  BINARY = ENV.fetch("SQLITE_DOCTOR") { File.expand_path("../../../target/debug/sqlite-doctor", __dir__) }
  REPOSITORY = File.expand_path("../../..", __dir__)
  AR_VERSION = ENV.fetch("AR_VERSION", "8.1.4")

  APP = <<~'RUBY'.freeze
    require "bundler/inline"
    version = ENV.fetch("AR_VERSION")
    gemfile(ENV["CI"] == "true" || ENV["INSTALL"] == "1", quiet: true) do
      source "https://rubygems.org"
      gem "railties", version
      gem "activerecord", version
      gem "sqlite3", version.start_with?("7.1") ? "~> 1.4" : ">= 2.1"
      gem "sqlite_doctor", path: ENV.fetch("SQLITE_DOCTOR_GEM"), require: false
    end
    require "rails"
    require "active_record/railtie"
    require "sqlite_doctor"

    class App < Rails::Application
      config.root = __dir__
      config.eager_load = false
      config.logger = Logger.new(nil)
      config.secret_key_base = "test"
      config.paths["db/migrate"] = ["db/custom_migrate"] if ENV["CUSTOM_MIGRATIONS"]
    end
    Rails.application.initialize!
    Rails.application.load_tasks
    task after: "sqlite_doctor:check" do
      puts "after ran"
    end
    Rake::Task["after"].invoke
  RUBY

  TABLES = <<~RUBY.freeze
    create_table :tenants
    create_table :parents do |t|
      t.integer :tenant_id
    end
    create_table :children do |t|
      t.references :parent, null: false, foreign_key: { on_delete: :cascade }
    end
  RUBY

  SCHEMA = <<~RUBY.freeze
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

  def setup
    skip "build sqlite-doctor or set SQLITE_DOCTOR" unless File.executable?(BINARY)
    @repository = Dir.mktmpdir("railtie-test")
    @root = File.join(@repository, "apps", "shop")
    FileUtils.mkdir_p(File.join(@root, "config"))
    database_yml
    File.write(File.join(@root, "app.rb"), APP)
  end

  def teardown
    FileUtils.remove_entry(@repository) if @repository
  end

  def database_yml(yaml = "development:\n  adapter: sqlite3\n  database: db/development.sqlite3\n")
    File.write(File.join(@root, "config", "database.yml"), yaml)
  end

  def migration(path, body)
    file = File.join(@root, path)
    FileUtils.mkdir_p(File.dirname(file))
    name = File.basename(file, ".rb").sub(/\A\d+_/, "").split("_").map(&:capitalize).join
    File.write(file, <<~RUBY)
      class #{name} < ActiveRecord::Migration[7.1]
        def change
          #{body}
        end
      end
    RUBY
  end

  def run_app(env = {})
    Open3.capture2e(
      { "AR_VERSION" => AR_VERSION, "SQLITE_DOCTOR_GEM" => REPOSITORY, "SQLITE_DOCTOR" => BINARY,
        "RAILS_ENV" => "development" }.merge(env),
      RbConfig.ruby, File.join(@root, "app.rb"), chdir: @root
    )
  end

  def commit_base
    system("git", "-C", @repository, "init", "-q", exception: true)
    system("git", "-C", @repository, "add", ".", exception: true)
    system("git", "-C", @repository, "-c", "user.name=t", "-c", "user.email=t@example.invalid",
           "commit", "-q", "-m", "base", exception: true)
  end

  def test_a_clean_check_lets_dependent_tasks_run
    migration("db/migrate/20260101000001_create_tables.rb", TABLES)
    File.write(File.join(@root, "db", "development.sqlite3"), "")

    output, status = run_app

    assert_equal 0, status.exitstatus, output
    assert_includes output, "1 pending migration(s):\n  db/migrate/20260101000001_create_tables.rb"
    assert_includes output, "after ran"
    tables, = Open3.capture2("sqlite3", File.join(@root, "db", "development.sqlite3"),
                             "SELECT count(*) FROM sqlite_schema")
    assert_equal "0", tables.strip, "the check must not migrate the configured database"
  end

  def test_base_with_custom_nested_migrations_in_a_subdirectory_app
    migration("db/custom_migrate/2026/20260101000001_create_tables.rb", TABLES)
    FileUtils.mkdir_p(File.join(@root, "db"))
    File.write(File.join(@root, "db", "schema.rb"), SCHEMA)
    commit_base
    migration("db/custom_migrate/20250101000002_link_tenants.rb", "add_foreign_key :parents, :tenants")

    output, status = run_app("BASE" => "HEAD", "CUSTOM_MIGRATIONS" => "1")

    assert_equal 1, status.exitstatus, output
    assert_includes output, "1 pending migration(s):\n  db/custom_migrate/20250101000002_link_tenants.rb\n"
    assert_match(%r{SQLD001: .*\n  --> db/custom_migrate/20250101000002_link_tenants\.rb}, output)
    refute_includes output, "after ran"
    refute File.exist?(File.join(@root, "db", "development.sqlite3")), "BASE must not touch the configured database"
  end

  def test_several_databases_need_a_choice
    database_yml(<<~YAML)
      development:
        primary:
          adapter: sqlite3
          database: db/development.sqlite3
        archive:
          adapter: sqlite3
          database: db/archive.sqlite3
          migrations_paths: db/archive_migrate
    YAML

    output, status = run_app

    assert_equal 2, status.exitstatus, output
    assert_includes output, "development migrates 2 databases (primary, archive); choose one with DATABASE=<name>"
  end

  def test_a_file_uri_is_refused_with_exit_2
    database_yml("development:\n  adapter: sqlite3\n  database: \"file:db/development.sqlite3?immutable=1\"\n")

    output, status = run_app

    assert_equal 2, status.exitstatus, output
    assert_includes output, "is a file: URI"
  end

  def test_a_database_that_is_not_a_file_is_an_error
    database_yml("development:\n  adapter: sqlite3\n  database: \":memory:\"\n")

    output, status = run_app

    assert_equal 2, status.exitstatus, output
    assert_includes output, "is not a file-backed database"
  end
end

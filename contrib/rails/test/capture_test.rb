require "bundler/inline"

ar_version = ENV.fetch("AR_VERSION", "~> 8.1")
gemfile(ENV["CI"] == "true") do
  source "https://rubygems.org"
  gem "activerecord", ar_version
  gem "sqlite3", ar_version.start_with?("7.1") ? "~> 1.4" : ">= 2.1"
  gem "minitest", "~> 5.0"
end

require "minitest/autorun"
require "tmpdir"
require_relative "../sqlite_doctor_capture"

ActiveRecord::Migration.verbose = false

class CaptureTest < Minitest::Test
  SCHEMA = <<~SQL.freeze
    CREATE TABLE tenants (id INTEGER PRIMARY KEY);
    CREATE TABLE parents (id INTEGER PRIMARY KEY, tenant_id INTEGER, name TEXT);
    CREATE TABLE children (id INTEGER PRIMARY KEY,
      parent_id INTEGER NOT NULL REFERENCES parents(id) ON DELETE CASCADE);
    CREATE INDEX idx_parents_name ON parents(name);
    INSERT INTO tenants VALUES (1);
    INSERT INTO parents VALUES (1, 1, 'a');
    INSERT INTO children VALUES (1, 1), (2, 1);
  SQL

  def setup
    @dir = Dir.mktmpdir("capture-test")
    @database = File.join(@dir, "app.sqlite3")
    SQLite3::Database.new(@database) { |db| db.execute_batch(SCHEMA) }
    @migrations = File.join(@dir, "migrate")
    Dir.mkdir(@migrations)
    @output = File.join(@dir, "capture.sql")
    ActiveRecord::Base.establish_connection(adapter: "sqlite3", database: @database)
  end

  def teardown
    ActiveRecord::Base.connection_handler.clear_all_connections!
    FileUtils.remove_entry(@dir)
  end

  def migration(body)
    File.write(File.join(@migrations, "20260101000001_change.rb"), <<~RUBY)
      class Change < ActiveRecord::Migration[7.1]
        def change
          #{body}
        end
      end
    RUBY
  end

  def capture
    SqliteDoctorCapture.capture(
      config: { adapter: "sqlite3", database: @database },
      migrations_paths: @migrations,
      output: @output
    )
  end

  def original_digest
    Digest::SHA256.file(@database).hexdigest
  end

  def test_captures_schema_pragma_and_rebuild_without_touching_the_original
    migration("add_foreign_key :parents, :tenants, column: :tenant_id")
    before = original_digest

    result = capture
    sql = File.read(@output)

    assert_nil result[:error]
    assert_equal before, original_digest
    assert_includes sql, "CREATE TABLE children"
    assert_includes sql, "PRAGMA foreign_keys = 1;"
    assert_match(/BEGIN.*TRANSACTION;/i, sql)
    assert_includes sql, 'DROP TABLE "parents";'
    assert_includes sql, "-- Observed on the copy: rows in children 2 -> 0"
    assert_equal ["children 2 -> 0"], result[:shrunk_tables]
  end

  def test_restores_the_callers_connection
    migration("add_column :parents, :extra, :text")
    capture

    assert_equal File.realpath(@database), File.realpath(ActiveRecord::Base.connection_db_config.database)
    assert_equal 1, SqliteDoctorCapture.base_connection.select_value("SELECT count(*) FROM parents")
  end

  def test_refuses_to_write_the_capture_over_the_database_or_its_side_files
    migration("add_column :parents, :extra, :text")
    before = original_digest

    [@database, "#{@database}-wal"].each do |target|
      error = assert_raises(ArgumentError) do
        SqliteDoctorCapture.capture(config: { adapter: "sqlite3", database: @database },
                                    migrations_paths: @migrations, output: target)
      end
      assert_match(/refusing to write/, error.message)
    end
    assert_equal before, original_digest
  end

  def test_refuses_when_another_pool_uses_the_original
    other = Class.new(ActiveRecord::Base) do
      self.abstract_class = true
      def self.name = "OtherRecord"
    end
    other.establish_connection(adapter: "sqlite3", database: @database)
    migration("add_column :parents, :extra, :text")

    error = assert_raises(RuntimeError) { capture }
    assert_match(/connection pool .* also uses/, error.message)
    refute File.exist?(@output)
  end

  def test_fails_when_a_migration_writes_the_original_through_its_own_connection
    migration(<<~RUBY)
      SQLite3::Database.new(#{@database.inspect}) { |db| db.execute("UPDATE parents SET name = 'changed'") }
      add_column :parents, :extra, :text
    RUBY

    error = assert_raises(RuntimeError) { capture }
    assert_match(/changed while migrating the copy/, error.message)
    refute File.exist?(@output)
  end

  def test_failed_statements_are_kept_as_comments
    migration(<<~RUBY)
      execute "INSERT INTO parents VALUES (2, 1, 'a')"
      execute "CREATE UNIQUE INDEX idx_unique_name ON parents(name)"
    RUBY

    result = capture
    sql = File.read(@output)

    refute_nil result[:error]
    assert_equal 1, result[:failed]
    assert_includes sql, "-- failed: CREATE UNIQUE INDEX idx_unique_name ON parents(name)"
    refute_match(/^CREATE UNIQUE INDEX idx_unique_name/, sql)
  end
end

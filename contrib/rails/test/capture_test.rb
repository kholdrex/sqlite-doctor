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

class SecondaryRecord < ActiveRecord::Base
  self.abstract_class = true
end

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

  def test_keeps_tables_whose_names_only_start_with_sqlite
    SQLite3::Database.new(@database) do |db|
      db.execute_batch(<<~SQL)
        CREATE TABLE sqliteX (id INTEGER PRIMARY KEY AUTOINCREMENT,
          parent_id INTEGER REFERENCES parents(id) ON DELETE CASCADE);
        INSERT INTO sqliteX (parent_id) VALUES (1);
      SQL
    end
    migration("add_foreign_key :parents, :tenants, column: :tenant_id")

    result = capture
    sql = File.read(@output)

    assert_includes sql, "CREATE TABLE sqliteX"
    refute_match(/^CREATE TABLE sqlite_sequence/, sql)
    assert_includes result[:shrunk_tables], "sqliteX 1 -> 0"
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

  def test_refuses_to_write_the_capture_over_another_configured_database
    migration("add_column :parents, :extra, :text")
    queue = File.join(@dir, "queue.sqlite3")
    SQLite3::Database.new(queue) { |db| db.execute("CREATE TABLE jobs (id INTEGER PRIMARY KEY)") }
    queue_digest = Digest::SHA256.file(queue).hexdigest
    previous = ActiveRecord::Base.configurations
    ActiveRecord::Base.configurations = {
      "test" => { "primary" => { "adapter" => "sqlite3", "database" => @database },
                  "queue" => { "adapter" => "sqlite3", "database" => queue } }
    }

    ["#{queue}-wal", queue].each do |target|
      error = assert_raises(ArgumentError, target) do
        SqliteDoctorCapture.capture(config: { adapter: "sqlite3", database: @database },
                                    migrations_paths: @migrations, output: target)
      end
      assert_match(/refusing to write the capture (over|to) .*#{Regexp.escape(File.basename(target))}/,
                   error.message)
    end

    ActiveRecord::Base.configurations = previous
    SecondaryRecord.establish_connection(adapter: "sqlite3", database: queue)
    error = assert_raises(ArgumentError) do
      SqliteDoctorCapture.capture(config: { adapter: "sqlite3", database: @database },
                                  migrations_paths: @migrations, output: queue)
    end
    assert_match(/refusing to write/, error.message)
    assert_equal queue_digest, Digest::SHA256.file(queue).hexdigest
  ensure
    ActiveRecord::Base.configurations = previous if previous
    SecondaryRecord.remove_connection
  end

  def test_a_trailing_line_comment_does_not_swallow_the_semicolon
    migration(<<~RUBY)
      execute "CREATE TABLE staging (id INTEGER PRIMARY KEY) -- staging table"
      execute "INSERT INTO staging SELECT id FROM parents;  -- copy  "
    RUBY

    capture
    sql = File.read(@output)

    assert_includes sql, "CREATE TABLE staging (id INTEGER PRIMARY KEY) -- staging table\n/**/;\n"
    assert_includes sql, "INSERT INTO staging SELECT id FROM parents;  -- copy\n/**/;\n"
  end

  def test_replaces_only_an_earlier_capture
    migration("add_column :parents, :extra, :text")
    other = File.join(@dir, "other.sqlite3")
    SQLite3::Database.new(other) { |db| db.execute("CREATE TABLE jobs (id INTEGER PRIMARY KEY)") }
    linked = File.join(@dir, "linked.sqlite3")
    File.symlink(other, linked)
    digest = Digest::SHA256.file(other).hexdigest

    [other, linked].each do |target|
      error = assert_raises(ArgumentError, target) do
        SqliteDoctorCapture.capture(config: { adapter: "sqlite3", database: @database },
                                    migrations_paths: @migrations, output: target)
      end
      assert_match(target == linked ? /symbolic link/ : /not an earlier capture/, error.message)
    end
    assert_equal digest, Digest::SHA256.file(other).hexdigest
    assert File.symlink?(linked)

    [File.join(@dir, "empty.sql"), File.join(@dir, "dangling.sql")].each_with_index do |target, index|
      index.zero? ? File.write(target, "") : File.symlink(File.join(@dir, "missing"), target)
      assert_raises(ArgumentError, target) do
        SqliteDoctorCapture.capture(config: { adapter: "sqlite3", database: @database },
                                    migrations_paths: @migrations, output: target)
      end
    end
    error = assert_raises(ArgumentError) do
      SqliteDoctorCapture.capture(config: { adapter: "sqlite3", database: @database },
                                  migrations_paths: @migrations,
                                  output: File.join(@dir, File.basename(@database).upcase))
    end
    assert_match(/refusing to write the capture over/, error.message)

    capture
    capture
    assert File.read(@output).start_with?(SqliteDoctorCapture::CAPTURE_HEADER)
  end

  def test_refuses_a_database_a_migration_creates_at_the_output
    migration(<<~RUBY)
      SQLite3::Database.new(#{@output.inspect}) { |db| db.execute("CREATE TABLE jobs (id INTEGER)") }
    RUBY

    error = assert_raises(ArgumentError) { capture }
    assert_match(/not an earlier capture/, error.message)
    names = nil
    SQLite3::Database.new(@output) { |db| names = db.execute("SELECT name FROM sqlite_schema") }
    assert_equal [["jobs"]], names
  end

  def test_comments_cannot_join_statements
    migration(<<~RUBY)
      execute "-- failed: an earlier attempt\nCREATE TABLE staging (id INTEGER PRIMARY KEY) -- staging"
      execute "CREATE TABLE notes (id INTEGER PRIMARY KEY) /* notes"
      execute 'CREATE TABLE "stage/*" (id INTEGER PRIMARY KEY)'
    RUBY

    capture
    sql = File.read(@output)

    assert_includes sql, "CREATE TABLE staging (id INTEGER PRIMARY KEY) -- staging\n/**/;\n"
    assert_includes sql, "CREATE TABLE notes (id INTEGER PRIMARY KEY) /* notes\n/**/;\n"
    assert_includes sql, "CREATE TABLE \"stage/*\" (id INTEGER PRIMARY KEY)\n/**/;\n"
  end

  def test_a_capture_is_private_and_a_replaced_one_keeps_its_mode
    migration("add_column :parents, :extra, :text")

    capture
    assert_equal 0o600, File.stat(@output).mode & 0o777

    File.chmod(0o640, @output)
    capture
    assert_equal 0o640, File.stat(@output).mode & 0o777
  end

  def test_refuses_when_another_pool_uses_the_original
    migration("add_column :parents, :extra, :text")

    [@database, "file:#{@database}", "file://#{@database}?mode=rw",
     "file:#{@database}?mode=memory&mode=rw", "file:#{@database}?mode=memory&%6dode=rw",
     "file://localhost#{@database.gsub('t', '%74')}#fragment"].each do |database|
      SecondaryRecord.establish_connection(adapter: "sqlite3", database: database)

      error = assert_raises(RuntimeError, database) { capture }
      assert_match(/connection pool .* also uses/, error.message, database)
      refute File.exist?(@output), database
    end
  ensure
    SecondaryRecord.remove_connection
  end

  def test_refuses_a_pool_whose_uri_names_another_host
    SecondaryRecord.establish_connection(adapter: "sqlite3", database: "file://elsewhere#{@database}")
    migration("add_column :parents, :extra, :text")

    error = assert_raises(RuntimeError) { capture }
    assert_match(/cannot tell which file/, error.message)
    refute File.exist?(@output)
  ensure
    SecondaryRecord.remove_connection
  end

  def test_allows_pools_on_in_memory_and_other_databases
    migration("add_column :parents, :extra, :text")

    [":memory:", "file::memory:?cache=shared", "file:shared?mode=memory&cache=shared",
     "file:#{File.join(@dir, 'other.sqlite3')}"].each do |database|
      SecondaryRecord.establish_connection(adapter: "sqlite3", database: database)

      assert_nil capture[:error], database
    end
  ensure
    SecondaryRecord.remove_connection
  end

  def test_sql_sent_to_another_database_is_not_captured
    SecondaryRecord.establish_connection(adapter: "sqlite3", database: File.join(@dir, "other.sqlite3"))
    migration(<<~RUBY)
      SecondaryRecord.connection_pool.with_connection do |connection|
        connection.transaction do
          connection.execute("CREATE TABLE elsewhere (id INTEGER)")
          connection.execute("PRAGMA foreign_keys = OFF")
        end
      end
      add_column :parents, :extra, :text
    RUBY

    result = capture
    sql = File.read(@output)

    assert_nil result[:error]
    assert_equal ["elsewhere"], SQLite3::Database.new(File.join(@dir, "other.sqlite3")) { |db|
      break db.execute("SELECT name FROM sqlite_schema").flatten
    }
    assert_includes sql, 'ALTER TABLE "parents" ADD "extra" text'
    refute_includes sql, "elsewhere"
    refute_includes sql, "PRAGMA foreign_keys = OFF"
  ensure
    SecondaryRecord.remove_connection
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

  def test_never_follows_a_planted_staging_file
    migration("add_column :parents, :extra, :text")
    before = original_digest
    real_open = File.method(:open)
    planted = []
    File.singleton_class.send(:define_method, :open) do |path, *args, **kw, &block|
      if path.to_s.end_with?(".tmp") && planted.empty?
        planted << path
        File.symlink(File.expand_path(path.to_s.sub(/[^\/]+\z/, "app.sqlite3")), path)
      end
      real_open.call(path, *args, **kw, &block)
    end

    capture
    assert_equal before, original_digest
    assert_equal 1, planted.size
    assert File.symlink?(planted.first)
  ensure
    File.singleton_class.send(:define_method, :open, real_open)
  end

  def test_a_wal_checkpoint_while_migrating_is_not_a_change
    SQLite3::Database.new(@database) { |db| db.execute("PRAGMA journal_mode = WAL") }
    holder = SQLite3::Database.new(@database)
    holder.execute("INSERT INTO tenants VALUES (2)")
    migration("add_column :parents, :extra, :text")
    before = original_digest
    checkpoint = nil
    run_migrations = SqliteDoctorCapture.method(:run_migrations)
    SqliteDoctorCapture.define_singleton_method(:run_migrations) do |*arguments|
      checkpoint = holder.execute("PRAGMA wal_checkpoint(FULL)").first
      run_migrations.call(*arguments)
    end

    result = capture

    assert_nil result[:error]
    busy, _, checkpointed_frames = checkpoint
    assert_equal 0, busy
    assert_operator checkpointed_frames, :>, 0
    refute_equal before, original_digest
  ensure
    SqliteDoctorCapture.define_singleton_method(:run_migrations, run_migrations)
    holder&.close
  end

  def test_leaves_activerecord_unconnected_when_it_started_unconnected
    ActiveRecord::Base.remove_connection
    migration("add_column :parents, :extra, :text")
    capture

    assert_raises(ActiveRecord::ConnectionNotEstablished) { ActiveRecord::Base.connection_db_config }
  end
end

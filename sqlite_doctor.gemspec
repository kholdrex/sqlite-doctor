require_relative "contrib/rails/lib/sqlite_doctor/version"

Gem::Specification.new do |spec|
  spec.name = "sqlite_doctor"
  spec.version = SqliteDoctor::VERSION
  spec.authors = ["Alex Kholodniak"]
  spec.summary = "Checks pending ActiveRecord migrations for SQLite table rebuilds that delete or change rows"
  spec.description = "Runs pending migrations on a copy of the SQLite database, records the SQL ActiveRecord " \
                     "ran, and scans it with the sqlite-doctor binary. Adds bin/rails sqlite_doctor:check."
  spec.homepage = "https://github.com/kholdrex/sqlite-doctor"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.1"
  spec.metadata = {
    "source_code_uri" => "https://github.com/kholdrex/sqlite-doctor",
    "changelog_uri" => "https://github.com/kholdrex/sqlite-doctor/blob/main/CHANGELOG.md",
    "rubygems_mfa_required" => "true"
  }
  spec.files = Dir["contrib/rails/lib/**/*.{rb,rake}"] + ["LICENSE", "README.md"]
  spec.require_paths = ["contrib/rails/lib"]
  spec.add_dependency "activerecord", ">= 7.1", "< 9"
  spec.add_dependency "sqlite3", ">= 1.4", "< 3"
end

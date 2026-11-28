module SqliteDoctor
  class Railtie < Rails::Railtie
    rake_tasks do
      load File.expand_path("../tasks/sqlite_doctor.rake", __dir__)
    end
  end
end

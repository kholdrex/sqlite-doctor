require_relative "sqlite_doctor/version"
require_relative "sqlite_doctor/capture"
require_relative "sqlite_doctor/check"
require_relative "sqlite_doctor/railtie" if defined?(Rails::Railtie)

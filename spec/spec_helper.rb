require 'simplecov'
require 'sample_metrics'
require 'temp_metrics_dir'

SimpleCov.start do
  add_filter 'fuzz'
  add_filter 'tmp'
  add_filter 'vendor/ruby'
end

RSpec.configure do |config|
  config.include SampleMetrics, :sample_metrics
  config.include TempMetricsDir, :temp_metrics_dir
  config.after(:all) do
    cleanup_temp_metrics_dir if defined?(cleanup_temp_metrics_dir)
  end
end



require 'prometheus/client/registry'
require 'prometheus/client/configuration'
require 'prometheus/client/mmaped_value'

module Prometheus
  # Client is a ruby implementation for a Prometheus compatible client.
  module Client
    class << self
      attr_writer :configuration

      def configuration
        @configuration ||= Configuration.new
      end

      def configure
        yield(configuration)
      end

      # Returns a default registry object
      def registry
        @registry ||= Registry.new
      end

      def logger
        configuration.logger
      end

      def pid
        configuration.pid_provider.call
      end

      # Resets the registry and reinitializes all metrics files.
      # Use case: clean up everything in specs `before` block,
      # to prevent leaking the state between specs which are updating metrics.
      def reset!
        @registry = nil
        ::Prometheus::Client::MmapedValue.reset_and_reinitialize
      end

      def cleanup!
        Dir.glob("#{configuration.multiprocess_files_dir}/*.db").each { |f| File.unlink(f) if File.exist?(f) }
      end

      # With `force: false`: reinitializes metric files only for processes with the changed PID.
      # With `force: true`: reinitializes all metrics files.
      # Always keeps the registry.
      # Use case  (`force: false`): pick up new metric files on each worker start,
      # without resetting already registered files for the master or previously initialized workers.
      def reinitialize_on_pid_change(force: false)
        if force
          ::Prometheus::Client::MmapedValue.reset_and_reinitialize
        else
          ::Prometheus::Client::MmapedValue.reinitialize_on_pid_change
        end
      end
    end
  end
end

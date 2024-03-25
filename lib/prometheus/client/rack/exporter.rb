# encoding: UTF-8

require 'prometheus/client'
require 'prometheus/client/formats/text'
require 'prometheus/client/formats/protobuf'

module Prometheus
  module Client
    module Rack
      # Exporter is a Rack middleware that provides a sample implementation of
      # a Prometheus HTTP client API.
      class Exporter
        attr_reader :app, :registry, :path

        FALLBACK = Formats::Text

        def initialize(app, options = {})
          @app = app
          @registry = options[:registry] || Client.registry
          @path = options[:path] || '/metrics'

          if Prometheus::Client.configuration.enable_protobuf && Prometheus::Client.configuration.rust_multiprocess_metrics
            @formats = [Formats::Text, Formats::Protobuf]
          else
            @formats = [Formats::Text]
          end
          @acceptable = build_dictionary(@formats, FALLBACK)
        end

        def call(env)
          if env['PATH_INFO'] == @path
            format = negotiate(env['HTTP_ACCEPT'], @acceptable)
            format ? respond_with(format) : not_acceptable(@formats)
          else
            @app.call(env)
          end
        end

        private

        def negotiate(accept, formats)
          accept = '*/*' if accept.to_s.empty?

          parse(accept).each do |content_type, _|
            return formats[content_type] if formats.key?(content_type)
          end

          nil
        end

        def parse(header)
          header.to_s.split(/\s*,\s*/).map do |type|
            attributes = type.split(/\s*;\s*/)
            quality = extract_quality(attributes)

            [attributes.join('; '), quality]
          end.sort_by(&:last).reverse
        end

        def extract_quality(attributes, default = 1.0)
          quality = default

          attributes.delete_if do |attr|
            quality = attr.split('q=').last.to_f if attr.start_with?('q=')
          end

          quality
        end

        def respond_with(format)
          response = if Prometheus::Client.configuration.value_class.multiprocess
                       format.marshal_multiprocess
                     else
                       format.marshal
                     end
          [
            200,
            { 'Content-Type' => format::CONTENT_TYPE },
            [response],
          ]
        end

        def not_acceptable(formats)
          types = formats.map { |format| format::MEDIA_TYPE }

          [
            406,
            { 'Content-Type' => 'text/plain' },
            ["Supported media types: #{types.join(', ')}"],
          ]
        end

        def build_dictionary(formats, fallback)
          formats.each_with_object('*/*' => fallback) do |format, memo|
            memo[format::CONTENT_TYPE] = format
            memo[format::MEDIA_TYPE] = format
          end
        end
      end
    end
  end
end

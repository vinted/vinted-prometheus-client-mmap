require 'prometheus/client/helper/json_parser'

module Prometheus
  module Client
    module Helper
      module EntryParser
        class ParsingError < RuntimeError;
        end

        MINIMUM_SIZE = 8
        START_POSITION = 8
        VALUE_BYTES = 8
        ENCODED_LENGTH_BYTES = 4

        def used
          slice(0..3).unpack('l')[0]
        end

        def parts
          @parts ||= File.basename(filepath, '.db')
                       .split('_')
                       .map { |e| e.gsub(/-\d+$/, '') } # remove trailing -number
        end

        def type
          parts[0].to_sym
        end

        def pid
          (parts[2..-1] || []).join('_')
        end

        def multiprocess_mode
          parts[1]
        end

        def empty?
          size < MINIMUM_SIZE || used.zero?
        end

        def entries(ignore_errors = false)
          return Enumerator.new {} if empty?

          Enumerator.new do |yielder|
            used_ = used # cache used to avoid unnecessary unpack operations

            pos = START_POSITION # used + padding offset
            while pos < used_ && pos < size && pos > 0
              data = slice(pos..-1)
              unless data
                raise ParsingError, "data slice is nil at pos #{pos}" unless ignore_errors
                pos += 8
                next
              end

              encoded_len, first_encoded_bytes = data.unpack('LL')
              if encoded_len.nil? || encoded_len.zero? || first_encoded_bytes.nil? || first_encoded_bytes.zero?
                # do not parse empty data
                pos += 8
                next
              end

              entry_len = ENCODED_LENGTH_BYTES + encoded_len
              padding_len = 8 - entry_len % 8

              value_offset = entry_len + padding_len # align to 8 bytes
              pos += value_offset

              if value_offset > 0 && (pos + VALUE_BYTES) <= size # if positions are safe
                yielder.yield data, encoded_len, value_offset, pos
              else
                raise ParsingError, "data slice is nil at pos #{pos}" unless ignore_errors
              end
              pos += VALUE_BYTES
            end
          end
        end

        def parsed_entries(ignore_errors = false)
          result = entries(ignore_errors).map do |data, encoded_len, value_offset, _|
            begin
              encoded, value = data.unpack(format('@4A%d@%dd', encoded_len, value_offset))
              [encoded, value]
            rescue ArgumentError => e
              Prometheus::Client.logger.debug("Error processing data: #{bin_to_hex(data[0, 7])} len: #{encoded_len} value_offset: #{value_offset}")
              raise ParsingError, e unless ignore_errors
            end
          end

          result.reject!(&:nil?) if ignore_errors
          result
        end

        def to_metrics(metrics = {}, ignore_errors = false)
          parsed_entries(ignore_errors).each do |key, value|
            begin
              metric_name, name, labelnames, labelvalues = JsonParser.load(key)
              labelnames ||= []
              labelvalues ||= []

              metric = metrics.fetch(metric_name,
                                     metric_name: metric_name,
                                     help: 'Multiprocess metric',
                                     type: type,
                                     samples: [])
              if type == :gauge
                metric[:multiprocess_mode] = multiprocess_mode
                metric[:samples] += [[name, labelnames.zip(labelvalues) + [['pid', pid]], value]]
              else
                # The duplicates and labels are fixed in the next for.
                metric[:samples] += [[name, labelnames.zip(labelvalues), value]]
              end
              metrics[metric_name] = metric

            rescue JSON::ParserError => e
              raise ParsingError(e) unless ignore_errors
            end
          end

          metrics.reject! { |e| e.nil? } if ignore_errors
          metrics
        end

        private

        def bin_to_hex(s)
          s.each_byte.map { |b| b.to_s(16) }.join
        end
      end
    end
  end
end

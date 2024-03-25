require 'fuzzbert'
require 'tempfile'
require 'prometheus/client'
require 'prometheus/client/helper/mmaped_file'
require 'fast_mmaped_file_rs'

module MmapedFileHelper
  def self.assert_equals(a, b)
    raise "#{a} not equal #{b}" unless a == b || a.is_a?(Float) && a.nan? && b.is_a?(Float) && b.nan?
  end

  def self.process(filepath)
    f = Prometheus::Client::Helper::MmapedFile.open(filepath)
    metrics = {}

    f.to_metrics(metrics, true)
    positions = {}
    f.entries(true).each do |data, encoded_len, value_offset, pos|
      encoded, value = data.unpack(format('@4A%d@%dd', encoded_len, value_offset))
      positions[encoded] = pos
      assert_equals(f.fetch_entry(positions, encoded, value - 1), value)
    end

    f.upsert_entry(positions, 'key', 0.1)
    assert_equals(f.fetch_entry(positions, 'key', 0), 0.1)

    f.upsert_entry(positions, 'key2', 0.2)
    assert_equals(f.fetch_entry(positions, 'key2', 0), 0.2)
  rescue Prometheus::Client::Helper::EntryParser::ParsingError
  rescue PrometheusParsingError
  ensure
    # fuzzbert wraps the process in a trap context. Ruby doesn't allow
    # mutexes to be synchronized in that context, so we need to spawn another
    # thread to close the file.
    Thread.new { f.close }.join(0.5)
  end
end

class PrintAndExitHandler
  def handle(error_data)
    puts error_data[:id]
    p error_data[:data]
    puts error_data[:pid]
    puts error_data[:status]
    Process.exit(1)
  end
end

fuzz 'MmapedFile' do
  deploy do |data|
    tmpfile = Tempfile.new('mmmaped_file')
    tmpfile.write(data)
    tmpfile.close

    MmapedFileHelper.process(tmpfile.path)

    tmpfile.unlink
  end

  data 'completely random' do
    FuzzBert::Generators.random
  end

  data 'should have 10000 bytes used and first entry' do
    c = FuzzBert::Container.new
    c << FuzzBert::Generators.fixed([10000, 0, 11, '[1,1,[],[]] ', 1].pack('LLLA12d'))
    c << FuzzBert::Generators.random(2)
    c << FuzzBert::Generators.fixed([0, 0].pack('CC'))
    c << FuzzBert::Generators.fixed('[1,1,[],[]]')
    c << FuzzBert::Generators.random
    c.generator
  end
end

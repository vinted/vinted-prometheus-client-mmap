require 'prometheus/client/mmaped_dict'
require 'prometheus/client/page_size'
require 'tempfile'

describe Prometheus::Client::MmapedDict do
  let(:tmp_file) { Tempfile.new('mmaped_dict') }
  let(:tmp_mmaped_file) { Prometheus::Client::Helper::MmapedFile.open(tmp_file.path) }

  after do
    tmp_mmaped_file.close
    tmp_file.close
    tmp_file.unlink
  end

  describe '#initialize' do
    describe "empty mmap'ed file" do
      it 'is initialized with correct size' do
        described_class.new(tmp_mmaped_file)

        expect(File.size(tmp_file.path)).to eq(tmp_mmaped_file.send(:initial_mmap_file_size))
      end
    end

    describe "mmap'ed file that is above minimum size" do
      let(:above_minimum_size) { Prometheus::Client::Helper::EntryParser::MINIMUM_SIZE + 1 }
      let(:page_size) { Prometheus::Client::PageSize.page_size }

      before do
        tmp_file.truncate(above_minimum_size)
      end

      it 'is initialized with the a page size' do
        described_class.new(tmp_mmaped_file)

        tmp_file.open
        expect(tmp_file.size).to eq(page_size);
      end
    end
  end

  describe 'read on boundary conditions' do
    let(:locked_file) { Prometheus::Client::Helper::MmapedFile.ensure_exclusive_file }
    let(:mmaped_file) { Prometheus::Client::Helper::MmapedFile.open(locked_file) }
    let(:page_size) { Prometheus::Client::PageSize.page_size }
    let(:target_size) { page_size }
    let(:iterations) { page_size / 32 }
    let(:dummy_key) { '1234' }
    let(:dummy_value) { 1.0 }
    let(:expected) { { dummy_key => dummy_value } }

    before do
      Prometheus::Client.configuration.multiprocess_files_dir = Dir.tmpdir

      data = described_class.new(Prometheus::Client::Helper::MmapedFile.open(locked_file))

      # This test exercises the case when the value ends on the last byte.
      # To generate a file like this, we create entries that require 32 bytes
      # total to store with 7 bytes of padding at the end.
      #
      # To make things align evenly against the system page size, add a dummy
      # entry that will occupy the next 3 bytes to start on a 32-byte boundary.
      # The filestructure looks like:
      #
      # Bytes 0-3  : Total used size of file
      # Bytes 4-7  : Padding
      # Bytes 8-11 : Length of '1234' (4)
      # Bytes 12-15: '1234'
      # Bytes 24-31: 1.0
      # Bytes 32-35: Length of '1000000000000' (13)
      # Bytes 36-48: '1000000000000'
      # Bytes 49-55: Padding
      # Bytes 56-63: 0.0
      # Bytes 64-67: Length of '1000000000001' (13)
      # Bytes 68-80: '1000000000001'
      # Bytes 81-87: Padding
      # Bytes 88-95: 1.0
      # ...
      data.write_value(dummy_key, dummy_value)

      (1..iterations - 1).each do |i|
        # Using a 13-byte string
        text = (1000000000000 + i).to_s
        expected[text] = i.to_f
        data.write_value(text, i)
      end

      data.close
    end

    it '#read_all_values' do
      values = described_class.read_all_values(locked_file)

      expect(values.count).to eq(iterations)
      expect(values).to match_array(expected.to_a)
    end
  end

  describe 'read and write values' do
    let(:locked_file) { Prometheus::Client::Helper::MmapedFile.ensure_exclusive_file }
    let(:mmaped_file) { Prometheus::Client::Helper::MmapedFile.open(locked_file) }

    before do
      Prometheus::Client.configuration.multiprocess_files_dir = Dir.tmpdir

      data = described_class.new(Prometheus::Client::Helper::MmapedFile.open(locked_file))
      data.write_value('foo', 100)
      data.write_value('bar', 500)

      data.close
    end

    after do
      mmaped_file.close if File.exist?(mmaped_file.filepath)
      Prometheus::Client::Helper::FileLocker.unlock(locked_file) if File.exist?(mmaped_file.filepath)
      File.unlink(locked_file) if File.exist?(mmaped_file.filepath)
    end

    it '#inspect' do
      data = described_class.new(Prometheus::Client::Helper::MmapedFile.open(locked_file))

      expect(data.inspect).to match(/#{described_class}:0x/)
      expect(data.inspect).not_to match(/@position/)
    end

    it '#read_all_values' do
      values = described_class.read_all_values(locked_file)

      expect(values.count).to eq(2)
      expect(values[0]).to eq(['foo', 100])
      expect(values[1]).to eq(['bar', 500])
    end

    it '#read_all_positions' do
      data = described_class.new(Prometheus::Client::Helper::MmapedFile.open(locked_file))

      positions = data.positions

      # Generated via https://github.com/luismartingarcia/protocol:
      # protocol "Used:4,Pad:4,K1 Size:4,K1 Name:4,K1 Value:8,K2 Size:4,K2 Name:4,K2 Value:8"
      #
      # 0                   1                   2                   3
      # 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
      # +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
      # |  Used |  Pad  |K1 Size|K1 Name|   K1 Value    |K2 Size|K2 Name|
      # +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
      # |  K2 Value   |
      # +-+-+-+-+-+-+-+
      expect(positions).to eq({ 'foo' => 16, 'bar' => 32 })
    end

    describe '#write_value' do
      it 'writes values' do
        # Reload dictionary
        #
        data = described_class.new(mmaped_file)
        data.write_value('new value', 500)
        # Overwrite existing values
        data.write_value('foo', 200)
        data.write_value('bar', 300)

        values = described_class.read_all_values(locked_file)

        expect(values.count).to eq(3)

        expect(values[0]).to eq(['foo', 200])
        expect(values[1]).to eq(['bar', 300])
        expect(values[2]).to eq(['new value', 500])
      end

      context 'when mmaped_file got deleted' do
        it 'is able to write to and expand metrics file' do
          data = described_class.new(mmaped_file)
          data.write_value('new value', 500)
          FileUtils.rm(mmaped_file.filepath)

          1000.times do |i|
            data.write_value("new new value #{i}", 567)
          end

          expect(File.exist?(locked_file)).not_to be_truthy
        end
      end
    end
  end
end

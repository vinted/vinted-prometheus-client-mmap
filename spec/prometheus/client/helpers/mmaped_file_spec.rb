require 'spec_helper'
require 'prometheus/client/helper/mmaped_file'
require 'prometheus/client/page_size'

describe Prometheus::Client::Helper::MmapedFile do
  let(:filename) { Dir::Tmpname.create('mmaped_file_') {} }

  after do
    File.delete(filename) if File.exist?(filename)
  end

  describe '.open' do
    it 'initialize PRIVATE mmaped file read only' do
      expect(described_class).to receive(:new).with(filename).and_call_original

      expect(described_class.open(filename)).to be_instance_of(described_class)
    end
  end

  context 'file does not exist' do
    let (:subject) { described_class.open(filename) }
    it 'creates and initializes file correctly' do
      expect(File.exist?(filename)).to be_falsey

      subject

      expect(File.exist?(filename)).to be_truthy
    end

    it 'creates a file with minimum initial size' do
      expect(File.size(subject.filepath)).to eq(subject.send(:initial_mmap_file_size))
    end

    context 'when initial mmap size is larger' do
      let(:page_size) { Prometheus::Client::PageSize.page_size }
      let (:initial_mmap_file_size) { page_size + 1024 }

      before do
        allow_any_instance_of(described_class).to receive(:initial_mmap_file_size).and_return(initial_mmap_file_size)
      end

      it 'creates a file with increased minimum initial size' do
        expect(File.size(subject.filepath)).to eq(page_size * 2);
      end
    end
  end

  describe '.ensure_exclusive_file' do
    let(:tmpdir) { Dir.mktmpdir('mmaped_file') }
    let(:pid) { 'pid' }

    before do
      allow(Prometheus::Client.configuration).to receive(:multiprocess_files_dir).and_return(tmpdir)
      allow(Prometheus::Client.configuration).to receive(:pid_provider).and_return(pid.method(:to_s))
    end

    context 'when no files are already locked' do
      it 'provides first possible filename' do
        expect(described_class.ensure_exclusive_file('mmaped_file'))
          .to match(/.*mmaped_file_pid-0\.db/)
      end

      it 'provides first and second possible filenames for two invocations' do
        expect(described_class.ensure_exclusive_file('mmaped_file'))
          .to match(/.*mmaped_file_pid-0\.db/)

        expect(described_class.ensure_exclusive_file('mmaped_file'))
          .to match(/.*mmaped_file_pid-1\.db/)
      end
    end

    context 'when first possible file exists for current file ID' do
      let(:first_mmaped_file) { described_class.ensure_exclusive_file('mmaped_file') }
      before do
        first_mmaped_file
      end

      context 'first file is unlocked' do
        before do
          Prometheus::Client::Helper::FileLocker.unlock(first_mmaped_file)
        end

        it 'provides first possible filename discarding the lock' do
          expect(described_class.ensure_exclusive_file('mmaped_file'))
            .to match(/.*mmaped_file_pid-0\.db/)
        end

        it 'provides second possible filename for second invocation' do
          expect(described_class.ensure_exclusive_file('mmaped_file'))
            .to match(/.*mmaped_file_pid-0\.db/)

          expect(described_class.ensure_exclusive_file('mmaped_file'))
            .to match(/.*mmaped_file_pid-1\.db/)
        end
      end

      context 'first file is not unlocked' do
        it 'provides second possible filename' do
          expect(described_class.ensure_exclusive_file('mmaped_file'))
            .to match(/.*mmaped_file_pid-1\.db/)
        end

        it 'provides second and third possible filename for two invocations' do
          expect(described_class.ensure_exclusive_file('mmaped_file'))
            .to match(/.*mmaped_file_pid-1\.db/)

          expect(described_class.ensure_exclusive_file('mmaped_file'))
            .to match(/.*mmaped_file_pid-2\.db/)
        end
      end
    end
  end
end

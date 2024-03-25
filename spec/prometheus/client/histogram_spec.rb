# encoding: UTF-8
require 'prometheus/client'
require 'prometheus/client/histogram'
require 'examples/metric_example'

describe Prometheus::Client::Histogram do
  before do
    allow(Prometheus::Client.configuration).to receive(:multiprocess_files_dir).and_return('tmp/')
  end

  let(:histogram) do
    described_class.new(:bar, 'bar description', {}, [2.5, 5, 10])
  end

  it_behaves_like Prometheus::Client::Metric do
    let(:type) { Hash }
  end

  describe '#initialization' do
    it 'raise error for unsorted buckets' do
      expect do
        described_class.new(:bar, 'bar description', {}, [5, 2.5, 10])
      end.to raise_error ArgumentError
    end

    it 'raise error for accidentally missing out an argument' do
      expect do
        described_class.new(:bar, 'bar description', [5, 2.5, 10])
      end.to raise_error Prometheus::Client::LabelSetValidator::InvalidLabelSetError
    end
  end

  describe '#observe' do
    it 'records the given value' do
      expect do
        histogram.observe({}, 5)
      end.to change { histogram.get }
    end

    xit 'raise error for le labels' do
      expect do
        histogram.observe({ le: 1 }, 5)
      end.to raise_error ArgumentError
    end
  end

  describe '#get' do
    before do
      histogram.observe({ foo: 'bar' }, 3)
      histogram.observe({ foo: 'bar' }, 5.2)
      histogram.observe({ foo: 'bar' }, 13)
      histogram.observe({ foo: 'bar' }, 4)
    end

    xit 'returns a set of buckets values' do
      expect(histogram.get(foo: 'bar')).to eql(2.5 => 0, 5 => 2, 10 => 3)
    end

    xit 'returns a value which responds to #sum and #total' do
      value = histogram.get(foo: 'bar')

      expect(value.sum).to eql(25.2)
      expect(value.total).to eql(4)
      expect(value.total_inf).to eql(4)
    end

    xit 'uses zero as default value' do
      expect(histogram.get({})).to eql(2.5 => 0, 5 => 0, 10 => 0)
    end
  end

  xdescribe '#values' do
    it 'returns a hash of all recorded summaries' do
      histogram.observe({ status: 'bar' }, 3)
      histogram.observe({ status: 'foo' }, 6)

      expect(histogram.values).to eql(
                                    { status: 'bar' } => { 2.5 => 0, 5 => 1, 10 => 1 },
                                    { status: 'foo' } => { 2.5 => 0, 5 => 0, 10 => 1 },
                                  )
    end
  end
end

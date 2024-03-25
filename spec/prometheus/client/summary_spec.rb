# encoding: UTF-8

require 'prometheus/client/summary'
require 'examples/metric_example'

describe Prometheus::Client::Summary do
  let(:summary) { Prometheus::Client::Summary.new(:bar, 'bar description') }

  it_behaves_like Prometheus::Client::Metric do
    let(:type) { Float }
  end

  describe '#observe' do
    it 'records the given value' do
      expect do
        summary.observe({}, 5)
      end.to change { summary.get }
    end
  end

  xdescribe '#get' do
    before do
      summary.observe({ foo: 'bar' }, 3)
      summary.observe({ foo: 'bar' }, 5.2)
      summary.observe({ foo: 'bar' }, 13)
      summary.observe({ foo: 'bar' }, 4)
    end

    it 'returns a set of quantile values' do
      expect(summary.get(foo: 'bar')).to eql(0.5 => 4, 0.9 => 5.2, 0.99 => 5.2)
    end

    it 'returns a value which responds to #sum and #total' do
      value = summary.get(foo: 'bar')

      expect(value.sum).to eql(25.2)
      expect(value.total).to eql(4)
    end

    it 'uses nil as default value' do
      expect(summary.get({})).to eql(0.5 => nil, 0.9 => nil, 0.99 => nil)
    end
  end

  xdescribe '#values' do
    it 'returns a hash of all recorded summaries' do
      summary.observe({ status: 'bar' }, 3)
      summary.observe({ status: 'foo' }, 5)

      expect(summary.values).to eql(
        { status: 'bar' } => { 0.5 => 3, 0.9 => 3, 0.99 => 3 },
        { status: 'foo' } => { 0.5 => 5, 0.9 => 5, 0.99 => 5 },
      )
    end
  end
end

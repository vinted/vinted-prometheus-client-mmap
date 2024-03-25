# encoding: UTF-8

require 'rack/test'
require 'prometheus/client/rack/collector'

describe Prometheus::Client::Rack::Collector do
  include Rack::Test::Methods

  before do
    allow(Prometheus::Client.configuration).to receive(:value_class).and_return(Prometheus::Client::SimpleValue)
  end

  let(:registry) do
    Prometheus::Client::Registry.new
  end

  let(:original_app) do
    ->(_) { [200, { 'Content-Type' => 'text/html' }, ['OK']] }
  end

  let!(:app) do
    described_class.new(original_app, registry: registry)
  end

  it 'returns the app response' do
    get '/foo'

    expect(last_response).to be_ok
    expect(last_response.body).to eql('OK')
  end

  it 'propagates errors in the registry' do
    counter = registry.get(:http_requests_total)
    expect(counter).to receive(:increment).and_raise(NoMethodError)

    expect { get '/foo' }.to raise_error(NoMethodError)
  end

  it 'traces request information' do
    # expect(Time).to receive(:now).and_return(Time.at(0.0), Time.at(0.2))
    labels = { method: 'get', host: 'example.org', path: '/foo', code: '200' }

    get '/foo'

    {
      http_requests_total: 1.0,
      # http_request_duration_seconds: { 0.5 => 0.2, 0.9 => 0.2, 0.99 => 0.2 }, # TODO: Fix summaries
    }.each do |metric, result|
      expect(registry.get(metric).get(labels)).to eql(result)
    end
  end

  context 'when the app raises an exception' do
    let(:original_app) do
      lambda do |env|
        raise NoMethodError if env['PATH_INFO'] == '/broken'

        [200, { 'Content-Type' => 'text/html' }, ['OK']]
      end
    end

    before do
      get '/foo'
    end

    it 'traces exceptions' do
      labels = { exception: 'NoMethodError' }

      expect { get '/broken' }.to raise_error NoMethodError

      expect(registry.get(:http_exceptions_total).get(labels)).to eql(1.0)
    end
  end

  context 'setting up with a block' do
    let(:app) do
      described_class.new(original_app, registry: registry) do |env|
        { method: env['REQUEST_METHOD'].downcase } # and ignore the path
      end
    end

    it 'allows labels configuration' do
      get '/foo/bar'

      labels = { method: 'get', code: '200' }

      expect(registry.get(:http_requests_total).get(labels)).to eql(1.0)
    end
  end
end

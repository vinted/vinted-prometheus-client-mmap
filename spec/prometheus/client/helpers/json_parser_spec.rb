require 'spec_helper'
require 'oj'
require 'prometheus/client/helper/json_parser'

describe Prometheus::Client::Helper::JsonParser do
  describe '.load' do
    let(:input) { %({ "a": 1 }) }

    shared_examples 'JSON parser' do
      it 'parses JSON' do
        expect(described_class.load(input)).to eq({ 'a' => 1 })
      end

      it 'raises JSON::ParserError' do
        expect { described_class.load("{false}") }.to raise_error(JSON::ParserError)
      end
    end

    context 'with Oj' do
      it_behaves_like 'JSON parser'
    end

    context 'without Oj' do
      before(:all) do
        Object.send(:remove_const, 'Oj')
        load File.join(__dir__,  "../../../../lib/prometheus/client/helper/json_parser.rb")
      end

      it_behaves_like 'JSON parser'
    end
  end
end

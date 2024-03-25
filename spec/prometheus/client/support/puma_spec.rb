require 'spec_helper'
require 'prometheus/client/support/puma'

class FakePumaWorker
  attr_reader :index

  def initialize(index)
    @index = index
  end
end

describe Prometheus::Client::Support::Puma do
  describe '.worker_pid_provider' do
    let(:worker_id) { '2' }
    let(:program_name) { $PROGRAM_NAME }

    subject(:worker_pid_provider) { described_class.worker_pid_provider }

    before do
      expect(described_class).to receive(:program_name)
        .at_least(:once)
        .and_return(program_name)
    end

    context 'when the current process is a Puma cluster worker' do
      context 'when the process name contains a worker id' do
        let(:program_name) { 'puma: cluster worker 2: 34740 [my-app]' }

        it { is_expected.to eq('puma_2') }
      end

      context 'when the process name does not include a worker id' do
        let(:worker_number) { 10 }

        before do
          stub_const('Puma::Cluster::Worker', FakePumaWorker)
          FakePumaWorker.new(worker_number)
        end

        it { is_expected.to eq("puma_#{worker_number}") }
      end
    end

    context 'when the current process is the Puma master' do
      let(:program_name) { 'bin/puma' }

      it { is_expected.to eq('puma_master') }
    end

    context 'when it cannot be determined that Puma is running' do
      let(:process_id) { 10 }

      before do
        allow(Process).to receive(:pid).and_return(process_id)
      end

      it { is_expected.to eq("process_id_#{process_id}") }
    end
  end
end

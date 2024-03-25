require 'spec_helper'
require 'prometheus/client/support/unicorn'

class FakeUnicornWorker
  attr_reader :nr

  def initialize(nr)
    @nr = nr
  end
end

describe Prometheus::Client::Support::Unicorn do
  describe '.worker_id' do
    let(:worker_id) { '09' }

    around do |example|
      old_name = $0
      example.run
      $0 = old_name
    end

    context 'process name contains worker id' do
      before do
        $0 = "program worker[#{worker_id}] arguments"
      end

      it 'returns worker_id' do
        expect(subject.worker_id).to eq(worker_id)
      end
    end

    context 'process name is without worker id' do
      it 'calls .object_based_worker_id id provider' do
        expect(subject).to receive(:object_based_worker_id).and_return(worker_id)

        expect(subject.worker_id).to eq(worker_id)
      end
    end
  end

  describe '.object_based_worker_id' do
    context 'when Unicorn is defined' do
      before do
        stub_const('Unicorn::Worker', FakeUnicornWorker)
      end

      context 'Worker instance is present in ObjectSpace' do
        let(:worker_number) { 10 }
        let!(:unicorn_worker) { FakeUnicornWorker.new(worker_number) }

        it 'Unicorn::Worker to be defined' do
          expect(defined?(Unicorn::Worker)).to be_truthy
        end

        it 'returns worker id' do
          expect(described_class.object_based_worker_id).to eq(worker_number)
        end
      end

      context 'Worker instance is not present in ObjectSpace' do
        it 'Unicorn::Worker id defined' do
          expect(defined?(Unicorn::Worker)).to be_truthy
        end

        it 'returns no worker id' do
          expect(ObjectSpace).to receive(:each_object).with(::Unicorn::Worker).and_return(nil)

          expect(described_class.object_based_worker_id).to eq(nil)
        end
      end
    end

    context 'Unicorn::Worker is not defined' do
      it 'Unicorn::Worker not defined' do
        expect(defined?(Unicorn::Worker)).to be_falsey
      end

      it 'returns no worker_id' do
        expect(described_class.object_based_worker_id).to eq(nil)
      end
    end
  end

  describe '.worker_pid_provider' do
    context 'worker_id is provided' do
      let(:worker_id) { 2 }
      before do
        allow(described_class).to receive(:worker_id).and_return(worker_id)
      end

      it 'returns worker pid created from worker id' do
        expect(described_class.worker_pid_provider).to eq("worker_id_#{worker_id}")
      end
    end

    context 'worker_id is not provided' do
      let(:process_id) { 10 }
      before do
        allow(described_class).to receive(:worker_id).and_return(nil)
        allow(Process).to receive(:pid).and_return(process_id)
      end

      it 'returns worker pid created from Process ID' do
        expect(described_class.worker_pid_provider).to eq("process_id_#{process_id}")
      end
    end
  end
end

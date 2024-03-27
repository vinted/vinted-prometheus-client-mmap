require 'bundler'
require 'rake/clean'
require 'rspec/core/rake_task'
require 'rubocop/rake_task'
require 'rake/extensiontask'
require 'gem_publisher'
require 'rb_sys'

cross_rubies = %w[3.3.0 3.2.0 3.1.0 3.0.0 2.7.0]
cross_platforms = %w[
  aarch64-linux
  arm64-darwin
  x86_64-darwin
  x86_64-linux
]

CLEAN.include FileList['**/*{.o,.so,.dylib,.bundle}'],
              FileList['**/extconf.h'],
              FileList['**/Makefile'],
              FileList['pkg/']

desc 'Default: run specs'
task default: [:spec]

# test alias
task test: :spec

desc 'Run specs'
RSpec::Core::RakeTask.new do |t|
  t.rspec_opts = '--require ./spec/spec_helper.rb'
end

desc 'Lint code'
RuboCop::RakeTask.new

Bundler::GemHelper.install_tasks

desc 'Publish gem to RubyGems.org'
task :publish_gem do |_t|
  gem = GemPublisher.publish_if_updated('prometheus-client-mmap.gemspec', :rubygems)
  puts "Published #{gem}" if gem
end

task :console do
  exec 'irb -r prometheus -I ./lib'
end

task :version do |_t|
  puts Prometheus::Client::VERSION
end

gemspec = Gem::Specification.load(File.expand_path('../prometheus-client-mmap.gemspec', __FILE__))

Gem::PackageTask.new(gemspec)

Rake::ExtensionTask.new('fast_mmaped_file_rs', gemspec) do |ext|
  ext.source_pattern = "*.{rs,toml}"
  ext.cross_compile = true
  ext.cross_platform = cross_platforms
end

namespace "gem" do
  task "prepare" do
    sh "bundle"
  end

  cross_platforms.each do |plat|
    desc "Build the native gem for #{plat}"
    task plat => "prepare" do
      require "rake_compiler_dock"

      ENV["RCD_IMAGE"] = "rbsys/#{plat}:#{RbSys::VERSION}"

      RakeCompilerDock.sh <<~SH, platform: plat
        (mkdir "$HOME/.protoc") ||: &&
        curl -L https://github.com/protocolbuffers/protobuf/releases/download/v24.4/protoc-24.4-linux-x86_64.zip -o "$HOME/.protoc/protoc-24.4-linux-x86_64.zip" &&
        cur="$(pwd)" &&
        cd "$HOME/.protoc"; unzip -o protoc-24.4-linux-x86_64.zip &&
        cd "$cur" &&
        export PATH="${PATH}:$HOME/.protoc/bin" &&
        bundle && \
        RUBY_CC_VERSION="#{cross_rubies.join(":")}" \
        rake native:#{plat} pkg/#{gemspec.full_name}-#{plat}.gem
      SH
    end
  end
end


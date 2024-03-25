## v1.1.1

- Cargo: Update dependencies for shlex security fix !149
- Revert merge request causing high committed RAM !151

## v1.1.0

- Add support for Ruby 3.3 in precompiled gems !144

## v1.0.3

- Don't publish Ruby gem with world-writeable files !146

## v1.0.2

- mmap: Use 'with_errno' helper to construct error !141

## v1.0.1

- file_info: Don't error out if file is expanded !139

## v1.0.0

- Make Rust the default extension and remove C extension !121
- Update to Rust 1.73.0 !135
- mmap: Detect unexpected file size changes !137

## v0.28.1

- Avoid file truncation in munmap !130
- ci: Disable testing of C extension !131

## v0.28.0

- Add helper to provide Puma worker PIDs !128

## v0.27.0

- Allow use of `instance` and `job` labels !125
- Fix header formatting issue !126
- cargo: Update to magnus 0.6.0 !127

## v0.26.1

- Use Kernel#warn instead of Prometheus::Client.logger.info when Rust extension not available !124

## v0.26.0

- Enable Rust extension by default !122
- ci: Fix address sanitizer jobs !123

## v0.25.0

- fix: make C and Rust extensions compatible with Ruby 3.3 RString changes !120

## v0.24.5

- file_entry: Use serde_json to parse entries !119

## v0.24.4

- ci: Run pages and ASAN on Ruby 3.1 !117
- parser: Don't assume values don't contain commas !118

## v0.24.3

- mmap: Use C types when interfacing with Ruby !116

## v0.24.2

- Start tracking shared child strings !114
- Convert 'type_' to non-static Symbol !115

## v0.24.1

- ci: Fix smoke test !113

## v0.24.0

- Expose Rust extension for use in marshaling metrics and read/write values !111
- Fix i386/debian CI job and refactor cache key handling !110

## v0.23.1

- Use c_long for Ruby string length !109

## v0.23.0

- Drop musl precompiled gem and relax RubyGems dependency !106

## v0.22.0

- Re-implement write path in Rust !103

## v0.21.0

- Remove 'rustc' check from 'Rakefile'  !97
- Add support for precompiled gems !99
- Refactor 'bin/setup' to remove uninitialized vars !100
- ci: create precompiled gems and push to Rubygems automatically !101
- Require RubyGems >= v3.3.22 !101

## v0.20.3

- Check for 'rustc' in 'extconf.rb' !95

## v0.20.2

- Allocate EntryMap keys only when needed !92
- Don't auto-install Rust toolchain on 'gem install' !93

## v0.20.1

- Install Rust extension to 'lib' !90

## v0.20.0

- Use system page size !84
- Implement 'to_metrics' in Rust !85

## v0.19.1

- No changes; v0.19.0 gem pulled in some unnecessary files.

## v0.19.0

- Fix seg fault after memory is unmapped !80

## v0.18.0

- pushgateway: add grouping_key feature !76

## v0.17.0

- Fix crash when trying to inspect all strings !74

## v0.16.2

- No code changes. Retagging due to extraneous file included in package.

## v0.16.1

- Improve LabelSetValidator debug messages !69
- Properly rescue Oj exceptions !70

## v0.16.0

- Make sure on reset we release file descriptors for open files !63

## v0.15.0

- Make labels order independent !60

## v0.14.0

- Remove deprecated taint mechanism logic !59

## v0.13.0

- Gauge: add decrement method to gauge metric type !57
- Update push.rb to use newest client_ruby code !56

## v0.12.0

- Remove deprecated rb_safe_level() and rb_secure() calls !53

## v0.11.0

- Include filename in IOError exception !47
- Fix clang-format violations !49
- CI: use libasan5 !50
- Truncate MmappedDict#inspect output !51

## v0.10.0

- Eliminate SIGBUS errors in parsing metrics !43
- Make it easier to diagnose clang-format failures !44
- Add Ruby 2.6 and 2.7 to CI builds !45

## v0.9.10

- Extend `Prometheus::Client.reinitialize_on_pid_change` method to receive `:force` param !40
  With `force: true` it reinitializes all metrics files.
  With `force: false` (default) it reinitializes only on changed PID (as it was before).
  In any case, it keeps the registry (as it was before).
  Thus, the change is backward compatible.

## v0.9.9

- Do not allow label values that will corrupt the metrics !38

## v0.9.7

- Restore Prometheus logger !36

## v0.9.7

- Disable warning if prometheus_multiproc_dir is not set !35

## v0.9.6

- Add missing `pid=` label for metrics without labels !31

## v0.9.5

- Set multiprocess_files_dir config to temp directory by default
  https://gitlab.com/gitlab-org/prometheus-client-mmap/merge_requests/28

# pominspect

A terminal UI for exploring a Maven build's effective POM and dependency tree.

`mvn help:effective-pom` and `mvn dependency:tree` give you thousands of lines, and they don't tell you *why* something is there. pominspect shows the same data as a navigable tree, and for every dependency it shows its **Origin**:

- which POM declared it, and on which line
- where its version came from: the POM itself, a parent, an imported BOM, or a `dependencyManagement` entry that overrode the requested version
- which path of dependencies pulled it in
- where else it appears

It also warns about **Version Overrides**: places where a library gets a different version than the dependency requiring it asked for. For example, `corporate-lib` was built against `aws-sdk` 2.x, but your build's BOM upgrades the SDK to 3.x. That kind of mismatch compiles fine and then fails at runtime.

```
┌ Modules ─────────────────────────────────────────┐┌ Detail ─────────────────────────────────────────────┐
│▾ multi (pom)                                     ││org.apache.commons:commons-lang3:3.14.0              │
│▾ web (jar) ⚠ 3                                   ││type: jar   scope: compile                           │
│  ▾ Dependencies (5 direct)                       ││not resolved here: omitted for conflict with 3.12.0  │
│    ▸ logback-classic 1.4.14  ch.qos.logback      ││                                                     │
│      commons-lang3 3.12.0  org.apache.commons    ││Version Override  ▼ Downgrade (warning)              │
│    ▾ commons-text 1.12.0  org.apache.commons     ││  requester  org.apache.commons:commons-text:1.12.0  │
│        commons-lang3 3.14.0 ▼ downgrade (gets 3. ││  asked      3.14.0                                  │
│    ▸ junit 4.13.2 [test]  junit                  ││  gets       3.12.0                                  │
│  ▸ Version Overrides (4) ⚠ 3                     ││  because    Mediation: a nearer path wins (web → c… │
│                                                  ││                                                     │
│                                                  ││Path                                                 │
│                                                  ││web → commons-text:1.12.0 → commons-lang3:3.14.0     │
└──────────────────────────────────────────────────┘└─────────────────────────────────────────────────────┘
 / search   w why   m maven log   r reload   ? help   q quit                               ✓ 3 modules
```

## Requirements

- A JDK and Maven. pominspect uses the project's `./mvnw` if there is one, otherwise `mvn` on your `PATH`.
- Rust, only if you build from source.

## Install

With Homebrew (macOS and Linux):

```sh
brew install raccoondev/tap/pominspect
```

Or download an archive for your platform from [Releases](https://github.com/RaccoonDev/pominspect/releases), unpack it, and put `pominspect` on your `PATH`. Builds are published for Linux (x86_64, arm64; static), macOS (Intel, Apple silicon) and Windows (x86_64), each with a `.sha256` checksum.

Or build from source:

```sh
cargo install --path .
```

Or run it without installing: `cargo run -- <path>`.

## Usage

```sh
pominspect                      # the Maven build in the current directory
pominspect path/to/project      # a directory containing pom.xml
pominspect path/to/pom.xml      # a specific POM
pominspect -P prod,cloud        # activate profiles, as with mvn -P
pominspect -Denv=staging        # set properties, as with mvn -D (repeatable)
pominspect --mvn /opt/maven/bin/mvn   # use a specific Maven executable
```

If you point it at the root of a multi-module build, every module is inspected. If you point it at a submodule, only that module is.

The screen opens straight away while Maven runs in the background; the status bar shows which module Maven is working on. Maven runs online, so it may download plugins and artifacts, exactly as it would if you ran it yourself. The first run on a fresh machine can take a while; press `m` to watch the Maven log.

`pominspect --help` lists all options.

## Reading the screen

**Left: the navigation tree.** Each module contains two entries: *Dependencies*, which holds the module's resolved dependency tree, and *Version Overrides* (see below).

| Marker | Meaning |
| --- | --- |
| `⚠ 3` next to a module | The module has 3 Version Override warnings |
| `▲ major (asked 2.4)` next to a dependency | A Version Override: the dependency requiring it asked for 2.4 and gets the version shown |
| `▼ downgrade (gets 3.12.0)` | The same, on a crossed-out entry that shows the *requested* version |
| crossed out | Maven left this entry out: a duplicate, or it lost to another version |
| `[test]`, `[provided]`, … | Scope, shown when it isn't `compile` |
| `✗ error` next to a module | Maven couldn't inspect this module; select it to see Maven's error |

**Right: details of the selected entry.**

- **Module:** coordinates, dependency counts, a summary of Version Overrides, and any Inspection Error.
- **Dependencies:** how many direct and resolved artifacts the module has.
- **A dependency:**
  - **Version Override**, if this entry is one: who asked for which version, what it gets, and why (see below).
  - **Origin**: `declared ←` gives the POM and line of a direct dependency; `pulled in ←` names the dependency that brought in a transitive one. `version ←` shows where the version came from. If `dependencyManagement` replaced the version, it shows both the requested and the managed version.
  - **Path** from the module down to this dependency.
  - **Other occurrences** of the same `groupId:artifactId` in the module, each marked as resolved (`✓`), omitted (`✗`), or as a Version Override (what its requester asked for and got).

## Version Overrides

A **Version Override** is a dependency that gets a different version than the artifact requiring it (its **requester**) asked for. There are two causes, both reported by Maven:

- **Mediation**: several paths ask for different versions and Maven keeps the nearest one (Maven calls the losers "omitted for conflict").
- **Management**: a `dependencyManagement` entry or an imported BOM replaces the version.

Each override is graded by comparing the two version numbers:

| Severity | Example | Warning? |
| --- | --- | --- |
| `▼` Downgrade | asked 3.14.0, gets 3.12.0 | yes: code built against the newer version may call methods that don't exist (`NoSuchMethodError`) |
| `▲` Major Upgrade | asked 2.4, gets 3.0 (below 1.0: asked 0.3, gets 0.4) | yes: likely breaking changes |
| `▲` Minor / Patch Upgrade | asked 2.0.7, gets 2.0.13 | no |
| `?` Unclassified | asked 1.0-RC1, gets 1.0 | no: the numbers match or don't parse, so pominspect doesn't guess |

Overrides that only affect `test` or `provided` scope keep their label but rank one level lower: a test-scope Downgrade is still a warning, a test-scope Major Upgrade isn't.

Version numbers are a heuristic, not proof: a major upgrade can be compatible, and a minor one can break. Treat a warning as "check this", not "this is broken". Libraries that changed their `groupId` between versions (like the AWS SDK's move from `com.amazonaws` to `software.amazon.awssdk`) look like two unrelated libraries to Maven, so no override is reported for them.

Each module's **Version Overrides** entry lists its overrides grouped into Downgrade, Major Upgrade, and everything else. Each row shows the library, the versions, and the requester. Press `Enter` on one to jump to it in the dependency tree, or `w` to see every path to that library in every module.

## Keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` | Move |
| `→` / `l` | Expand, or step into |
| `←` / `h` | Collapse, or go to the parent |
| `Enter` / `Space` | Toggle expand; on a Version Override, jump to it in the dependency tree |
| `E` / `C` | Expand / collapse everything below the selection |
| `PgUp` `PgDn` `g` `G` | Page up / down, top, bottom |
| `/` | Search by `group:artifact:version` (any part, case-insensitive) |
| `n` / `N` | Next / previous match |
| `w` | **Why**: every path to the selected dependency, in every module |
| `m` | Show / hide the Maven log |
| `r` | Run Maven again (e.g. after editing a POM) |
| `?` | Help |
| `q` / `Ctrl-C` | Quit |

## How it works

pominspect does not reimplement Maven. It runs Maven once, with pinned versions of two plugins, so what you see always matches what Maven does:

```
mvn -B -fae -f <pom> \
  org.apache.maven.plugins:maven-help-plugin:3.5.1:effective-pom -Dverbose -Doutput=<tmp>/effective-pom.xml \
  org.apache.maven.plugins:maven-dependency-plugin:3.8.1:tree -Dverbose -DoutputFile=<tmp>/deps.txt -DappendOutput=true
```

Then it parses both files: the effective POM's `-Dverbose` comments give file and line locations, and the verbose tree gives paths, conflicts and managed versions. Pinning the plugin versions keeps the output format stable, whatever versions your project configures. See [docs/adr/0001-delegate-model-building-to-maven.md](docs/adr/0001-delegate-model-building-to-maven.md) for why.

Because of this, pominspect can only show what Maven reports. For example, Maven doesn't explain *why* a profile was activated.

## Development

```sh
cargo test                 # unit tests, using captured Maven output in tests/fixtures/maven-output
cargo test -- --ignored    # also runs real Maven against tests/fixtures/multi
```

CI ([.github/workflows/ci.yml](.github/workflows/ci.yml)) runs on every push and pull request. It checks `cargo fmt` and `clippy -D warnings`, runs the tests on Linux, macOS and Windows, and runs the real-Maven test on Linux.

To release, set `version` in `Cargo.toml`, commit, then push a matching tag:

```sh
git tag v0.0.1 && git push origin v0.0.1
```

[.github/workflows/release.yml](.github/workflows/release.yml) then runs CI, builds the archives for every platform, publishes them as a GitHub release with generated notes, and updates the formula in [RaccoonDev/homebrew-tap](https://github.com/RaccoonDev/homebrew-tap) (using the `HOMEBREW_TAP_TOKEN` secret). It fails if the tag doesn't match the version in `Cargo.toml`.

[GLOSSARY.md](GLOSSARY.md) defines the project's terms (Effective POM, Origin, Inspection Error, …), and [docs/adr/](docs/adr/) records the main design decisions.

## License

[MIT](LICENSE)

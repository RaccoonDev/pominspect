# pominspect

A terminal UI for exploring a Maven build's effective POM and dependency tree.

`mvn help:effective-pom` and `mvn dependency:tree` give you thousands of lines, and they don't tell you *why* something is there. pominspect shows the same data as a navigable tree, and for every dependency it shows its **Origin**:

- which POM declared it, and on which line
- where its version came from: the POM itself, a parent, an imported BOM, or a `dependencyManagement` entry that overrode the requested version
- which path of dependencies pulled it in
- where else it appears, and which version won a conflict

```
┌ Modules ─────────────────────────────────────────┐┌ Detail ─────────────────────────────────────────────┐
│▾ multi (pom)                                     ││org.apache.commons:commons-lang3:3.14.0              │
│▾ web (jar) ⚠ 1                                   ││type: jar   scope: compile                           │
│  ▾ Dependencies (5 direct)                       ││not resolved here: omitted for conflict with 3.12.0  │
│    ▸ logback-classic 1.4.14  ch.qos.logback      ││                                                     │
│      commons-lang3 3.12.0 ⚠  org.apache.commons  ││Origin                                               │
│    ▾ commons-text 1.12.0  org.apache.commons     ││  pulled in  ← by org.apache.commons:commons-text... │
│        commons-lang3 3.14.0 ✗ conflict, 3.12.0 w ││  version    ← as requested by org.apache.commons... │
│    ▸ junit 4.13.2 [test]  junit                  ││                                                     │
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

**Left: the navigation tree.** Each module contains a *Dependencies* entry, which holds the module's resolved dependency tree.

| Marker | Meaning |
| --- | --- |
| `⚠ 2` next to a module | The module has 2 dependencies with version conflicts |
| `⚠` next to a dependency | This is the version that won a conflict |
| `✗ conflict, X wins` | Maven left this version out because version X won |
| crossed out, `(duplicate)` | Already present elsewhere in the tree, so not counted again here |
| `[test]`, `[provided]`, … | Scope, shown when it isn't `compile` |
| `✗ error` next to a module | Maven couldn't inspect this module; select it to see Maven's error |

**Right: details of the selected entry.**

- **Module:** coordinates, dependency counts, version conflicts, and any Inspection Error.
- **Dependencies:** how many direct and resolved artifacts the module has, and its version conflicts.
- **A dependency:**
  - **Origin**: `declared ←` gives the POM and line of a direct dependency; `pulled in ←` names the dependency that brought in a transitive one. `version ←` shows where the version came from. If `dependencyManagement` replaced the version, it shows both the requested and the managed version.
  - **Path** from the module down to this dependency.
  - **Other occurrences** of the same `groupId:artifactId` in the module, each marked as resolved (`✓`) or omitted (`✗`).

## Keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` | Move |
| `→` / `l` | Expand, or step into |
| `←` / `h` | Collapse, or go to the parent |
| `Enter` / `Space` | Toggle expand |
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

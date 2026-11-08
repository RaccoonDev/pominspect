# Delegate model building to Maven

pominspect does not reimplement Maven's model building (inheritance, BOM imports, interpolation, profile activation, dependency mediation). It invokes Maven and parses its output, then re-presents it. Correctness must always match what Maven itself does, and reimplementing those semantics in Rust would drift from Maven over time.

## Consequences

- A JVM and Maven (or the project's Maven wrapper) are required at runtime.
- Each inspection pays Maven's startup cost; results should be cached or computed lazily.
- pominspect can only show Origin information that Maven's output exposes (e.g. it shows a profile's declared activation, not why it fired).
- Maven plugins are invoked by fully pinned coordinates and write to output files, so the parsed format is stable regardless of the project's own plugin configuration.
- Maven runs online and downloads whatever it needs, exactly as if the user had run the command themselves.

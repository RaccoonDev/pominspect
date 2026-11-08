# Delegate model building to Maven

pominspect does not reimplement Maven's model building (inheritance, BOM imports, interpolation, profile activation, dependency mediation). It invokes Maven and parses its output, then re-presents it. Correctness must always match what Maven itself does, and reimplementing those semantics in Rust would drift from Maven over time.

## Consequences

- A JVM and Maven (or the project's Maven wrapper) are required at runtime.
- Each inspection pays Maven's startup cost; results should be cached or computed lazily.
- pominspect can only show Origin information that Maven's output exposes (e.g. it shows a profile's declared activation, not why it fired).
- Maven plugins are invoked by fully pinned coordinates and write to output files, so the parsed format is stable regardless of the project's own plugin configuration.
- Maven runs online and downloads whatever it needs, exactly as if the user had run the command themselves.

## Exception: grading the Severity of Version Overrides

Maven reports *that* a Version Override happened (mediation or management) but not how risky it is, and it doesn't expose its version ordering. pominspect therefore compares versions itself, with a simplified numeric-segment heuristic rather than a port of Maven's `ComparableVersion`. Versions it can't parse are graded Unclassified rather than guessed. This comparison only grades Severity; it never decides which version is resolved, which remains Maven's job.

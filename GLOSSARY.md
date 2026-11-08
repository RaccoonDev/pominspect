# pominspect

A tool that makes a Maven project's build configuration understandable to a human (and, via structured output, to automated consumers), turning Maven's large, cryptic effective model into focused insight.

## Language

**POM**:
A single `pom.xml` file as written on disk, before any inheritance or interpolation.
_Avoid_: pom file, project file

**Effective POM**:
The fully computed model of a project after inheritance, imports, interpolation and profile activation have been applied.
_Avoid_: resolved POM, merged POM

**Origin**:
The explanation of why an element of the Effective POM is present and has its value: the POM, BOM, profile, property or dependency path it came from.
_Avoid_: source, provenance

**Transitive Dependency**:
A dependency the project receives through another dependency rather than declaring it itself.
_Avoid_: transient dependency, indirect dependency

**Inspection Error**:
A failed attempt by Maven to produce the Effective POM or dependency tree for a module; other modules remain browsable.
_Avoid_: build failure, crash

**Local Repository**:
The artifact cache on the user's machine (by default `~/.m2/repository`) that Maven reads and populates.
_Avoid_: m2, cache

**Remote Repository**:
A network artifact source (such as Maven Central or a corporate mirror) that Maven may download from while pominspect inspects a project.
_Avoid_: registry, server

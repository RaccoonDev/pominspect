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

### Version Overrides

**Requester**:
The artifact whose POM declares a dependency, i.e. the parent of that dependency in the dependency tree (the module itself for direct dependencies).
_Avoid_: parent, consumer, dependent

**Version Override**:
A single Requester-to-library edge where the build resolves a different version than the Requester asked for, identified by the Requester's coordinates and the library regardless of how many paths lead to it. Two Requesters overridden on the same library are two Version Overrides.
_Avoid_: conflict, version clash, mismatch

**Mediation**:
Maven's choice of one version when several paths request different versions of the same library; the losing requests are what Maven labels "omitted for conflict". One cause of a Version Override.
_Avoid_: conflict resolution

**Management**:
The replacement of a requested version by a `dependencyManagement` entry or an imported BOM. The other cause of a Version Override.
_Avoid_: pinning, forcing

**Severity**:
The estimated risk of a Version Override, from highest to lowest: Downgrade, Major Upgrade, Minor Upgrade, Patch Upgrade; Unclassified when the versions can't be compared. Overrides that only affect test or provided scope keep their Severity label but rank one level lower when deciding whether they are warnings. Downgrade and Major Upgrade are warnings; the rest are informational.
_Avoid_: priority, level

### Inspection

**Inspection Error**:
A failed attempt by Maven to produce the Effective POM or dependency tree for a module; other modules remain browsable.
_Avoid_: build failure, crash

**Local Repository**:
The artifact cache on the user's machine (by default `~/.m2/repository`) that Maven reads and populates.
_Avoid_: m2, cache

**Remote Repository**:
A network artifact source (such as Maven Central or a corporate mirror) that Maven may download from while pominspect inspects a project.
_Avoid_: registry, server

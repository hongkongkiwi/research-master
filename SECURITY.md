# Security dependency status

The repository build uses a local patch of citationberg 0.7.0 to require quick-xml 0.41.0. This removes RUSTSEC-2026-0194 and RUSTSEC-2026-0195 from the resolved dependency graph. The vendored source and license notices are unchanged; its manifest records the dependency change and upstream provenance.

## Distribution limitation

Cargo ignores dependency-level patches and removes `[patch]` overrides from registry packages. A crates.io release or a downstream library build without the same override would therefore resolve registry citationberg 0.7.0 and its affected quick-xml 0.38 dependency.

Crates.io publication is suspended in both Cargo.toml and the release workflow. Repository-root builds and prebuilt binaries retain the fixed graph. This containment does not remediate existing crates.io releases or downstream library consumers.

Library consumers must apply the same citationberg override in their own workspace root or wait for a registry-resolvable patched dependency chain. Do not treat the clean repository audit as evidence that an unpatched downstream graph is fixed.

Restore registry publication only after a released dependency chain requires fixed quick-xml and the normalized registry package has been checked without local patches. At that point, remove the vendored override, restore the Cargo publication setting and release job, and rerun dependency audits and citation tests.

## Maintenance notices

The dependency audit also reports unmaintained notices for paste, unsafe-libyaml, and ttf-parser. These notices remain visible and are not advisory suppressions or demonstrated application exploits.

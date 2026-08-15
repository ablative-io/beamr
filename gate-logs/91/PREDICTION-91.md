# PREDICTION — #91 battery (pre-registered BEFORE any leg output)

Registered: 2026-08-15, before battery launch, before pre-battery check
output was read. Base: main 70f61f7 + one doc-only edit (rustdoc comment
on `Module::mfa_at_ip`, crates/beamr/src/module.rs — the F3 window
amendment). Doc-only delta ⇒ zero behavioral change predicted.

NAMED AXES (per the axes rider — result-lines / passed / failed /
ignored; result-lines = per-binary `test result:` lines):

| leg                | result-lines | passed | failed | ignored |
|--------------------|--------------|--------|--------|---------|
| tests              | 75           | 2116   | 0      | 0       |
| tests-all-features | 75           | 2126   | 0      | 0       |

All 8 legs rc 0. Pin stable (HEAD sha unchanged pre/post). Census 0
untracked/modified in scored scope pre and post (tree clean at
registration; the one edit will be committed only AFTER green — battery
runs on the working tree with the doc edit as the sole modification, so
tree census will show exactly 1 modified path: crates/beamr/src/module.rs,
declared here up front, identical pre/post).

Verdict source: the per-leg rc tsv. The runner's COMPLETE marker is NOT
the verdict (#78/#84 law).

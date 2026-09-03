# Test deletion receipt

Each row used a fresh external `git archive HEAD` export. To let history-dependent tests run, the export was initialized as a disposable repository and fetched refs only from our first-party local mirror. The named artefact alone was removed, then the full suite was run; every removed artefact caused at least one guarding test to fail.

| artefact | observed failing tests |
| --- | --- |
| `unsafe-sites.json` | `test_site_kinds_partition`, `test_safety_lookback_counts`, `test_region_tally_sums_to_code_sites` |
| `registry-population.json` | `test_registry_untagged_split` |
| `reconciliation.json` | `test_astgrep_pack_tally`, `test_let_underscore_product_count` |
| `jit-walk-edges.json` | `test_jit_helper_surface_is_44`, `test_walk_bound_is_honoured` |
| `jit-crossings.json` | `test_no_regrade_in_r4_artefacts`, `test_r4_tally_is_derived_from_rows`, `test_r4_every_bounded_row_is_ruled`, `test_r4_real_rows_are_findings` |
| `jit-rulings.json` | `test_no_regrade_in_r4_artefacts`, `test_r4_every_bounded_row_is_ruled` |
| `jit-candidates-main.json` | `test_rf006_blind_to_cfg_test_controls`, `test_r4_raw_candidates_are_auditable` |
| `r6c-arm1-shape-hunt.txt` | `test_ar1_shape_hunt_receipt_matches_live_tail` |
| `r6c-arm3-carrier.json` | `test_r6c_arm3_carrier_receipt` |
| advisory sentence changed in an export to contain the cleared phrase | `test_advisory_phrase_cleared` |

The row-derived region test was additionally run after (a) removing one `sites` row and (b) editing `tallies.regions`; it failed in both mutations, so neither side is a duplicate constant.

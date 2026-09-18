# Ledger: model catalog end-to-end

TASK: Add `Provider::catalog()` and `ModelCatalog` so the `/model` picker lists real provider models, with bounds and a disk cache, and pass every test named in `SPEC-choose-a-model-and-configure-a-run`.

| id | requirement | test | verify command | state |
|----|-------------|------|----------------|-------|
| C1 | `ModelDescriptor` has only `id` and `display_name` | `descriptor_has_only_id_and_label` | `cargo test -p rho-core descriptor_has_only_id_and_label` | closed |
| C2 | `Usage` carries optional reasoning tokens that default to `None` | `reasoning_tokens_defaults_to_none` | `cargo test -p rho-core --test model` | closed |
| C3 | `Provider` exposes `catalog()` returning `Option<&dyn ModelCatalog>` | `catalog_reports_some_for_bedrock`, `catalog_reports_some_for_openrouter`, `catalog_reports_none_for_azure` | `cargo test -p rho-provider-bedrock catalog_reports_some_for_bedrock && cargo test -p rho-provider-openrouter catalog_reports_some_for_openrouter && cargo test -p rho-provider-azure catalog_reports_none_for_azure` | closed |
| C4 | `ModelCatalog::list_models` is bounded by count | `a_listing_over_the_cap_is_refused_and_names_both_numbers` | `cargo test -p rho-provider-openrouter a_listing_over_the_cap_is_refused_and_names_both_numbers` | closed |
| C5 | Bedrock lists foundation models with cancellation support | `catalog_reports_some_for_bedrock` | `cargo test -p rho-provider-bedrock catalog_reports_some_for_bedrock` | closed |
| C6 | OpenRouter lists models over its HTTP endpoint | `openrouter_lists_models_from_the_data_field`, `empty_list_is_ok_not_none`, `list_models_stops_on_cancel` | `cargo test -p rho-provider-openrouter openrouter_lists_models_from_the_data_field` | closed |
| C7 | Azure returns `None` from `catalog()` | `catalog_reports_none_for_azure` | `cargo test -p rho-provider-azure catalog_reports_none_for_azure` | closed |
| C8 | Cache is keyed by provider fingerprint and honours a 24-hour expiry | `cache_key_changes_with_region`, `stale_cache_is_not_served_as_fresh` | `cargo test -p rho-cli cache_key_changes_with_region` | closed |
| C8b | Stale cache entries are shown in the picker, marked stale, while a fresh list loads | `stale_cache_is_marked_stale` | `cargo test -p rho-tui --test model_picker stale_cache_is_marked_stale` | closed |
| C9 | Picker shows current model before the list returns and recovers from listing failure | `picker_shows_current_model_before_list_returns`, `listing_failure_keeps_the_current_model` | `cargo test -p rho-tui picker_shows_current_model_before_list_returns` | closed |
| C9b | A listing failure renders as exactly one error line in the picker panel | `listing_failure_shows_one_error_line` | `cargo test -p rho-tui --test render listing_failure_shows_one_error_line` | closed |
| C10 | Typed `/model <id>` works even when the list omits it | `typed_id_absent_from_list_is_accepted` | `cargo test -p rho-tui typed_id_absent_from_list_is_accepted` | closed |
| C11 | Model change writes a `ModelChange` record and takes effect next request | `model_change_writes_a_model_change_record`, `model_change_takes_effect_next_request`, `unchanged_selection_does_not_write_a_model_change_record` | `cargo test -p rho-core --test selection` | closed |
| C11b | Interactive TUI records the session | n/a | n/a | open | A `ModelChange` record needs a `SessionRecorder`. The interactive TUI path does not open a `Recording` today. This is a larger feature than this ledger. |
| C12 | Model switch drops reasoning bound to the old model | `model_switch_drops_reasoning_bound_to_old_model` | `cargo test -p rho-provider-bedrock model_switch_drops_reasoning_bound_to_old_model` | closed |
| C13 | `/speed fast` sets effort off; `/speed normal` restores start effort | `speed_fast_sets_effort_off`, `speed_normal_restores_start_effort` | `cargo test -p rho-tui --test model_picker speed` | closed |
| C14 | The provider contract harness checks every provider answers `catalog()` without panic | `run_all_checks_catalog_answer` in openrouter, bedrock, azure contract tests | `cargo test -p rho-provider-openrouter run_all_checks_catalog_answer && cargo test -p rho-provider-bedrock run_all_checks_catalog_answer && cargo test -p rho-provider-azure run_all_checks_catalog_answer` | closed |
| C15 | Live drive: `/model` opens a populated picker against Bedrock | manual | `./target/release/rho run "say hi" --provider bedrock --model ...`, press `/model` | open |

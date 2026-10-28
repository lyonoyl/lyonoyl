# Defenses

Every defense Lyonoyl Mail has, each beside what holds it: a test that fails without it, or a file that refuses its absence—`scripts/check-layering.sh` for the rules a test cannot hold, and `scripts/preflight.sh`, which runs both before every commit. A test named here is a Rust test unless a test file is named instead. `src/app/defenses.test.ts` fails when a row names a test or a file the tree no longer has. What each defense is against, and what is not covered, is in [the threat model](threat-model.md).

## A message cannot run code

| Defense | Held by |
| --- | --- |
| A message's HTML is cleaned in Rust, and the window parses none of it | `scripts/check-layering.sh`, `no_message_in_the_corpus_produces_anything_that_runs`, `a_message_written_only_in_html_still_gets_a_preview_and_it_is_text` |
| The body reaches the frame only through the application's own scheme, never as text across IPC and never through `srcdoc` | `scripts/check-layering.sh`, `every_answer_carries_the_headers_that_make_it_safe_to_look_at` |
| Every answer of that scheme carries its own policy: no script, no network, no frame, nothing but its own pictures | `the_policy_says_no_to_everything_and_spells_out_the_three_style_directives`, `what_is_not_a_document_is_served_under_a_policy_that_permits_nothing` |
| The message sits in `<iframe sandbox="" referrerpolicy="no-referrer">`, with no `allow-*` | `src/features/message-reader/MessageBody.test.tsx`, `scripts/check-layering.sh` |
| The application's own policy is never `null`, in development too, and lets no frame or picture from the web in | `scripts/check-layering.sh` |
| Each render answers to a name of its own, 32 random bytes, only while its message is open | `names_are_never_shared_between_renders`, `a_name_answers_while_the_message_is_open_and_not_after` |
| A script cannot be put into every frame, under any of the names Tauri ships that ability under | `scripts/check-layering.sh` |
| The window holds exactly the permissions it uses, and none by default | `src/shared/ipc/capability.test.ts` |
| A link opens only after its real address is shown, read by the same parser that opens it | `the_host_a_person_is_asked_about_is_the_host_that_opens`, `an_address_nothing_can_read_is_not_offered` |
| `unsafe` Rust stands in one file, for one call into AppKit | `scripts/check-layering.sh` |

## The mail is not lost

| Defense | Held by |
| --- | --- |
| Mail is removed here only on the server's exact report, or within a window it listed whole; an answer cut short removes nothing | `unknown_never_deletes`, `a_window_bounds_what_a_listing_can_delete`, `a_listing_refused_halfway_removes_nothing`, `a_listing_cut_off_by_the_connection_ending_removes_nothing` |
| A mass deletion in a large folder is put to the person first | `a_mass_deletion_in_a_large_folder_is_referred_to_the_user` |
| A folder the server renumbered is read again rather than emptied | `a_changed_generation_forces_a_resync_instead_of_deletions`, `a_mailbox_that_renumbered_is_not_read_by_a_number_from_before` |
| A removal reported for the folder being closed is never applied to the next | `what_left_one_mailbox_is_not_taken_for_what_left_the_next` |
| A message is named by an id of its own, never by the number a server gave it | `a_message_has_no_uid_column` |
| What arrives is written together with how far the folder was read, in one transaction | `arrivals_are_applied_together_with_the_new_cursor` |
| Drafts, the outbox, and the settings are written with `synchronous = FULL`, and every write begins `IMMEDIATE` | `applies_pragmas_to_every_pooled_connection`, `scripts/check-layering.sh` |
| A message is in the outbox before the call that wrote it answers | `a_message_is_committed_before_the_call_that_wrote_it_answers` |
| A message a server took is never handed over again | `a_message_is_taken_to_be_handed_over_once`, `a_message_a_round_took_and_never_finished_is_never_handed_over_again` |
| A message the outbox gives back returns to the drafts whole, with its files | `a_message_that_came_back_brings_the_files_it_carried`, `a_message_comes_back_whole_or_not_at_all` |
| What is being written is kept as a draft when the application ends | `a_note_is_written_down_as_a_draft_when_the_application_ends` |
| A connection opened to read cannot change a mailbox, whatever command it is handed, and no connection makes, renames, or deletes a mailbox, names a message by its position, or expunges one it did not name | `a_command_that_changes_the_server_never_leaves_the_machine`, `nothing_may_replace_a_message_whatever_it_was_opened_for_or_names_it_by`, `a_connection_opened_to_read_opens_a_mailbox_only_to_read_it`, `nothing_may_make_unmake_or_rename_a_mailbox_whatever_it_was_opened_for`, `even_a_connection_opened_to_act_may_not_name_a_message_by_its_position`, `a_bare_expunge_is_refused_on_every_connection_there_is` |
| A move or a delete reaches only a folder of the message's own account | `a_folder_is_only_found_in_the_account_that_owns_it` |
| A database a newer version wrote is refused, never set aside, and left byte for byte as it was, with the log beside it | `a_profile_a_newer_build_wrote_is_refused_with_its_own_code_and_left_alone`, `a_file_a_newer_build_wrote_is_refused_and_left_at_its_version` |

## Secrets stay secret

| Defense | Held by |
| --- | --- |
| Passwords and the database key live only in the system's keychain, or on a Linux desktop without one in a file a master password opens | `scripts/check-layering.sh` |
| Every page of every database is encrypted, and a file opens only under the key it was written with | `a_file_does_not_open_under_a_key_it_was_not_written_under` |
| Message bodies are sealed again inside that, under a key of their own | `another_key_does_not_open_it`, `bytes_that_were_never_sealed_do_not_open` |
| The data folder and its databases are this person's alone | `a_database_is_unreadable_by_anybody_else_from_the_first_run`, `the_data_directory_is_this_persons_alone_however_it_was_made` |
| What SQLite sorts stays in memory, and what it frees is overwritten | `keeps_what_it_sorts_in_memory_and_overwrites_what_it_frees` |
| A new key is never made over databases that exist | `a_store_that_says_it_holds_no_key_over_its_databases_is_not_given_one` |
| A secret moved in the keychain is written before the old item goes | `a_moved_secret_is_written_before_the_item_it_was_read_from_goes` |
| No library in the binary can log what it sends: logging is compiled out | `nothing_in_this_binary_can_be_made_to_write_down_what_it_sends` |
| The log holds numbers and words this build chose, never a word of a message | `nothing_a_message_wrote_reaches_the_journal` |
| A password crosses a connection in the clear only to this machine | `a_server_off_this_machine_set_to_plain_text_is_refused_before_anything_is_opened`, `plain_text_may_go_to_this_machine_and_nowhere_else` |

## Nothing leaks

| Defense | Held by |
| --- | --- |
| Pictures from the web load only when asked, and the frame itself reaches no network | `src/features/message-reader/MessageBody.test.tsx`, `the_policy_says_no_to_everything_and_spells_out_the_three_style_directives` |
| A picture is fetched by the core, naming the product alone, with no cookies and no referrer | `the_request_says_nothing_about_this_machine` |
| No address outside the public internet is asked, every redirect is judged again, and none leaves encryption | `every_address_that_is_not_the_public_internet_is_refused_by_the_shipped_client`, `a_redirect_to_this_machine_is_refused_and_nothing_is_sent_there`, `a_redirect_never_leaves_encryption` |
| An IPv4 address carried inside an IPv6 one is judged as what it carries | `an_ipv4_address_carried_inside_an_ipv6_one_is_judged_as_what_it_carries` |
| On macOS, checking a certificate fetches nothing the certificate names | `the_evaluation_is_made_to_fetch_nothing_before_it_runs` |
| Other people's addresses never cross into the window to decide what it draws | `the_addresses_themselves_do_not_cross_the_boundary` |
| A reply carries a bounded chain of references, and a message's boundary says nothing about anybody | `a_chain_that_fits_is_kept_whole_and_one_more_costs_the_name_after_the_oldest`, `a_message_with_a_file_is_cut_by_a_boundary_that_says_nothing_about_anybody` |
| A sent message leaves nothing of itself on this device once its copy is filed | `a_message_whose_copy_is_filed_leaves_nothing_of_itself_behind_once_read_back` |
| A copy looks for an update only after a yes, at most once a day, and its request carries no version | `a_fresh_install_looks_for_no_update_until_somebody_says_it_may`, `a_copy_that_asked_recently_waits`, `the_manifest_and_its_seal_are_asked_for_naming_the_product_alone` |
| A server is greeted as `[127.0.0.1]`, and a message is named after its sender's domain, never the machine | `the_server_is_greeted_with_a_name_that_says_nothing_about_this_machine`, `a_message_is_named_after_the_sender_rather_than_the_machine` |
| Where an address's mail lives is answered here first, asking nobody | `an_account_here_at_the_same_domain_answers_before_anybody_is_asked`, `a_provider_this_build_carries_is_answered_without_asking_anybody` |

## Files

| Defense | Held by |
| --- | --- |
| A file is saved only through the system's dialog, run by the core: no path comes from the window | `scripts/check-layering.sh` |
| A saved file's name is made rather than taken: every character that reorders text goes, and a name Windows keeps for a device is refused | `every_character_that_reorders_text_is_removed`, `a_name_windows_keeps_for_a_device_is_not_offered` |
| A saved file never keeps a leading dot | `a_space_between_the_dots_does_not_keep_one_of_them` |
| A saved file carries the mark of a download on macOS | `a_file_saved_out_of_a_message_carries_the_mark_of_a_download` |
| A link under the chosen name is replaced, never written through | `a_link_under_the_chosen_name_is_replaced_rather_than_written_through` |
| A file's row shows its real name and size, not the type its sender declared | `a_file_crosses_as_where_it_sits_what_it_is_called_and_how_large_it_is` |
| A file chosen to attach is read within a bound, and a device or a pipe is refused | `a_file_heavier_than_a_message_may_be_is_refused_as_it_is_chosen`, `a_device_that_never_ends_is_refused_rather_than_read`, `a_pipe_is_refused_without_waiting_for_somebody_to_write_into_it` |

## A message cannot exhaust the application

| Defense | Held by |
| --- | --- |
| A message larger than this build reads is refused before it is parsed | `a_message_larger_than_this_build_reads_is_refused_rather_than_parsed` |
| Nesting is measured before parsing, and a document deeper than an engine builds is not drawn | `a_document_as_deep_as_an_engine_builds_is_drawn_and_one_level_deeper_is_not` |
| A message of more than 8,000 elements is shown as text | `a_message_of_tens_of_thousands_of_elements_is_shown_as_text` |
| A message the parser panics on costs only itself, and the pass goes on | `a_message_the_parser_panics_on_is_read_once_and_whoever_asked_goes_on`, `scripts/check-layering.sh` |
| An answer from a mail server is bounded in what it makes the application hold | `an_answer_past_what_it_may_hold_is_refused` |
| A host that trickles is given up on, and so is a server that never greets | `a_host_that_keeps_trickling_is_given_up_on_all_the_same`, `a_server_that_takes_the_connection_and_never_greets_is_given_up_on` |

## Writing and sending

| Defense | Held by |
| --- | --- |
| Nothing a caller supplies can become a second header | `nothing_a_caller_supplies_can_become_a_second_header` |
| A message goes only as an account on this device, from an address that account owns | `a_message_from_one_account_cannot_be_sent_under_another`, `a_message_claiming_an_address_the_account_does_not_own_is_refused` |
| A message is bounded in size and in recipients before anything is committed | `a_message_too_large_to_write_down_is_refused_before_anything_is_committed`, `a_message_addressed_to_more_people_than_this_build_writes_is_refused` |
| A kept draft is bounded, and so are an account's drafts together, in number and in weight | `the_three_lines_of_addresses_are_bounded_together`, `an_account_keeps_as_many_drafts_as_it_may_and_writes_no_more`, `a_draft_that_would_take_the_drafts_past_what_they_may_weigh_is_refused_whole` |

## Updates

| Defense | Held by |
| --- | --- |
| A copy takes only a manifest under the project's seal, and only files the seal covers—the archive it names, held to its size and SHA-256 | `a_manifest_without_its_seal_is_not_handed_over`, `a_file_with_no_seal_on_it_is_refused`, `a_file_that_was_changed_after_it_was_sealed_is_refused`, `only_the_archive_the_manifest_names_is_taken`, `a_different_file_and_nothing_else_is_said_to_be_not_ours` |
| An archive comes only from the releases folder, under a plain name, and only the version offered is installed | `a_file_that_is_not_a_plain_name_in_the_releases_folder_is_refused`, `a_bundle_of_another_version_than_the_one_offered_is_refused` |
| The replacement is checked by `codesign` against the running copy's own requirement over every file it seals, never compared as text, and a copy signed ad hoc is never replaced | `a_bundle_carrying_the_running_copys_requirement_word_for_word_is_refused`, `a_bundle_changed_after_it_was_signed_is_refused`, `a_file_added_to_a_signed_bundle_is_refused`, `a_running_copy_signed_ad_hoc_is_not_one_to_replace` |
| An archive unpacks within a ceiling, and a bundle that needs a newer system is refused | `an_archive_that_expands_past_the_ceiling_is_stopped_on_the_way`, `a_bundle_asking_for_a_newer_system_than_this_mac_is_refused` |

## The repository

| Defense | Held by |
| --- | --- |
| Every check here runs before every commit, on the machine making it, and nothing runs anywhere else | `scripts/preflight.sh` |
| Each prohibition is refused by what it does rather than how it is spelled, and every rule a planted file can trip is fed one it must refuse | `scripts/check-layering.sh`, `scripts/check-layering.test.sh` |
| Every license the application ships is collected from the tree, the C code inside crates included, and none stops the product being sold | `nothing_in_the_tree_is_licensed_in_a_way_this_product_cannot_be_sold_under`, `a_refused_licence_is_caught_in_either_spelling`, `scripts/collect-vendored-licences.mjs` |
| Nothing in the tree identifies anybody | `scripts/check-anonymity.py` |
| Advisories against what the tree holds are read on every run | `scripts/preflight.sh` |

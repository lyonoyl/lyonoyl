use super::licences::{NoticeDto, COLLECTED};

fn collected() -> Vec<NoticeDto> {
    serde_json::from_str(COLLECTED).expect("the list shipped with this build is readable")
}

#[test]
fn the_list_shipped_with_this_build_is_readable() {
    // Embedded rather than fetched, so a build either has it or does not compile — but a build
    // that embedded something unreadable would compile and fail in front of somebody.
    assert!(!collected().is_empty());
}

#[test]
fn every_notice_says_what_it_covers_and_says_it_word_for_word() {
    // A notice with nothing under it is a row nobody can act on; one with no text is not a notice.
    // Both would pass a check that only counted rows.
    for notice in collected() {
        assert!(!notice.used.is_empty(), "{} covers nothing", notice.licence);
        assert!(
            notice.text.len() > 40,
            "{} has no text worth the name",
            notice.licence
        );
    }
}

#[test]
fn nothing_in_the_tree_is_licensed_in_a_way_this_product_cannot_be_sold_under() {
    // The rule from AGENTS.md section 6, checked against what is actually shipped rather than
    // against what somebody remembers agreeing to. A copyleft licence reaching the tree is a
    // product that cannot be sold, discovered at the moment it matters least.
    for notice in collected() {
        assert!(
            !refused(&notice.licence),
            "{} cannot be shipped by a product that is sold: {:?}",
            notice.licence,
            notice.used
        );
    }
}

/// Whether a licence is one this product may not ship under, however it is written down.
///
/// Two spellings reach the list, because two tools fill it. `cargo about` emits human names —
/// "GNU General Public License v3.0", "Mozilla Public License 2.0" — so a check for `GPL` matches
/// nothing at all there. The npm collector emits the identifier the package declares — `GPL-3.0`,
/// `AGPL-3.0-only` — so a check for "General Public License" matches nothing at all *there*. Each
/// form alone is a gate over half the tree.
fn refused(licence: &str) -> bool {
    const WRITTEN_OUT: [&str; 5] = [
        "General Public License",
        "Server Side Public License",
        "Business Source License",
        "Commons Clause",
        "Sustainable Use License",
    ];
    // Inside the identifier rather than at its start: `AGPL-3.0-only` and `LGPL-2.1-or-later` are
    // the forms that actually arrive, and a prefix test lets both of them through. Upper case
    // because that is what an identifier is; nothing permissive holds any of these letters.
    const IDENTIFIED: [&str; 5] = ["GPL", "SSPL", "BUSL", "CC-BY-NC", "PolyForm"];

    WRITTEN_OUT.iter().any(|one| licence.contains(one))
        || IDENTIFIED.iter().any(|one| licence.contains(one))
}

#[test]
fn a_refused_licence_is_caught_in_either_spelling() {
    // The check above reads a list somebody else's tools filled in, so it cannot be proved by the
    // list passing: a list with nothing refused in it passes whatever the check does. These are
    // the two spellings as the two tools actually write them.
    for written in [
        "GNU General Public License v3.0",
        "GNU Affero General Public License v3.0",
        "GNU Lesser General Public License v2.1",
        "Server Side Public License",
        "Business Source License 1.1",
    ] {
        assert!(refused(written), "{written} went through as sellable");
    }

    for identified in [
        "GPL-3.0",
        "AGPL-3.0-only",
        "LGPL-2.1-or-later",
        "SSPL-1.0",
        "BUSL-1.1",
    ] {
        assert!(refused(identified), "{identified} went through as sellable");
    }

    for permissive in [
        "MIT",
        "Apache-2.0",
        "Apache-2.0 OR MIT",
        "Apache License 2.0",
        "Mozilla Public License 2.0",
        "MPL-2.0",
        "ISC",
        "BSD-3-Clause",
        "OFL-1.1",
        "Unicode-3.0",
        "Zlib",
        "0BSD",
    ] {
        assert!(
            !refused(permissive),
            "{permissive} was refused and should not be"
        );
    }
}

#[test]
fn the_two_halves_of_the_application_are_both_in_it() {
    // Collected by two different tools from two different trees, and a script that quietly
    // stopped running one of them would leave half the notices missing.
    let all = collected();
    let names: Vec<&str> = all
        .iter()
        .flat_map(|notice| notice.used.iter().map(String::as_str))
        .collect();

    assert!(names.contains(&"tauri"), "the core's half");
    assert!(names.contains(&"react"), "the window's half");
}

#[test]
fn what_ships_without_declaring_itself_still_carries_its_notice() {
    // Neither is a crate, and neither is declared by the crate that builds it: `libsqlite3-sys`
    // says MIT about its own wrapper while compiling in Zetetic's SQLCipher, and `openssl-src`
    // says MIT/Apache-2.0 while compiling in OpenSSL. `cargo about` reads the declared field and
    // so sees neither, and both licences ask for the notice to travel with the binary.
    let all = collected();
    let covers = |what: &str| {
        all.iter()
            .any(|notice| notice.used.iter().any(|used| used.contains(what)))
    };

    assert!(covers("SQLCipher"), "SQLCipher is in every database file");
    assert!(
        covers("OpenSSL"),
        "OpenSSL is where SQLCipher takes its ciphers from"
    );
    // Not compiled in but copied in, which comes to the same thing for a notice: the files in
    // `src/shared/ui/primitives/` and the variants in `src/index.css` are somebody else's, and the
    // command that produced them is a tool rather than a dependency, so nothing in the npm half
    // declares them as shipping.
    assert!(
        covers("shadcn"),
        "the interface primitives in the tree are somebody else's code"
    );

    // The text rather than only the name: a notice whose body went missing is a notice that does
    // not travel, whatever the list says it covers.
    let zetetic = all
        .iter()
        .find(|notice| notice.used.iter().any(|used| used.contains("SQLCipher")))
        .expect("the notice is there");
    assert!(
        zetetic.text.contains("Zetetic"),
        "the SQLCipher notice carries no copyright line"
    );
}

#[test]
fn a_package_npm_names_twice_in_its_tree_is_in_it_once() {
    // `npm ls` prints a package's own dependencies only where it first meets the package, and a
    // bare version everywhere else. A collector that stops at the first sighting misses everything
    // under the one it skipped whenever the bare one comes first: the scheduler under `react-dom`,
    // Floating UI under the popper, and the dialog's scroll lock and `tslib` under the dialog.
    let all = collected();
    let covers = |what: &str| {
        all.iter()
            .any(|notice| notice.used.iter().any(|used| used == what))
    };

    for shipped in [
        "scheduler",
        "@floating-ui/dom",
        "react-remove-scroll-bar",
        "use-sidecar",
        "get-nonce",
        "tslib",
    ] {
        assert!(
            covers(shipped),
            "{shipped} ships in the window without its notice"
        );
    }
}

#[test]
fn a_build_tool_whose_output_ships_is_in_it_with_what_ships() {
    // Left out with the rest of the build tools, both would ship without a notice: the base styles
    // in the window's stylesheet are Tailwind's, and the module preload polyfill in its script is
    // vite's. Named by what of them ships, because the name alone reads as the whole tool.
    let all = collected();
    for tool in ["tailwindcss (", "vite ("] {
        assert!(
            all.iter()
                .any(|notice| notice.used.iter().any(|used| used.starts_with(tool))),
            "{tool} ships part of itself in the window without its notice"
        );
    }
}

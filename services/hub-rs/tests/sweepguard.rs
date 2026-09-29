#[path = "support/sweepguard.rs"]
mod sweepguard;
use sweepguard::Tally;

#[test]
fn verdict_floors_refuse_empty_and_half_empty_sweeps() {
    let mut tally = Tally::default();
    assert!(tally.require_both("corpus").is_err());
    assert!(tally.require_deny("corpus").is_err());
    for _ in 0..42 {
        tally.ran("allow");
    }
    assert!(tally.require_both("corpus").unwrap_err().contains("0 deny"));
    tally.ran("deny");
    tally.require_both("corpus").unwrap();
    let mut denies = Tally::default();
    for _ in 0..7 {
        denies.ran("refuse");
    }
    assert!(
        denies
            .require_both("corpus")
            .unwrap_err()
            .contains("0 allow")
    );
    denies.require_deny("corpus").unwrap();
}

#[test]
fn vocabulary_and_skip_diagnostics_keep_verdict_classes_separate() {
    let mut tally = Tally::default();
    for word in [
        "allow", "accept", " OK ", "PASS", "deny", "refuse", "reject", " Fail ", "whatever",
    ] {
        tally.ran(word);
    }
    assert_eq!(
        (tally.allow, tally.deny, tally.other, tally.executed()),
        (4, 4, 1, 9)
    );
    for reason in [
        "needsSymlinks",
        "needsSymlinks",
        "needsSymlinks",
        "needsHome",
        " ",
    ] {
        tally.skip(reason);
    }
    assert_eq!(tally.enumerated(), 14);
    let error = tally.require("corpus", 5, 5).unwrap_err();
    for expected in [
        "5 case(s) skipped",
        "needsSymlinks×3",
        "needsHome×1",
        "unspecified×1",
    ] {
        assert!(error.contains(expected), "{error}");
    }
    assert!(error.find("needsHome").unwrap() < error.find("needsSymlinks").unwrap());
    assert!(
        tally
            .to_string()
            .contains("9 cases (4 allow, 4 deny, 1 other)")
    );
}

#[test]
fn enumeration_ratchet_and_execution_floor_are_independent() {
    let mut tally = Tally::default();
    tally.ran("allow");
    tally.ran("deny");
    tally.require_both("corpus").unwrap();
    let error = tally.require_corpus("corpus", 79, 1, 1).unwrap_err();
    for expected in ["reached 2 cases", "floor is 79", "SHRANK"] {
        assert!(error.contains(expected));
    }
    for _ in 0..77 {
        tally.skip("needsSymlinks");
    }
    tally.require_corpus("corpus", 79, 1, 1).unwrap();
    assert!(tally.require_every("all cases", 79).is_err());
    tally.require_every("executed", 2).unwrap();
    let mut skipped = Tally::default();
    for _ in 0..79 {
        skipped.skip("needsSymlinks");
    }
    assert!(skipped.require_corpus("corpus", 79, 1, 1).is_err());
}

#[test]
fn real_path_population_mutations_cannot_satisfy_sweep_floors() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/path-containment-cases.json"
    ))
    .unwrap();
    for (name, rows, floor, allow, deny) in [
        ("paths", &fixture["cases"], 8, 3, 5),
        ("filenames", &fixture["sessionFilenames"]["cases"], 12, 3, 9),
    ] {
        let rows = rows.as_array().unwrap();
        for mutation in [
            "none",
            "empty",
            "allow-only",
            "deny-only",
            "skip-denies",
            "drop-one",
        ] {
            let mut tally = Tally::default();
            for (index, row) in rows.iter().enumerate() {
                let verdict = row["expect"].as_str().unwrap();
                let accepts = matches!(verdict, "allow" | "accept");
                if mutation == "empty"
                    || (mutation == "drop-one" && index == 0)
                    || (mutation == "allow-only" && !accepts)
                    || (mutation == "deny-only" && accepts)
                {
                    continue;
                }
                if mutation == "skip-denies" && !accepts {
                    tally.skip("host gate");
                } else {
                    tally.ran(verdict);
                }
            }
            assert_eq!(
                tally.require_corpus(name, floor, allow, deny).is_ok(),
                mutation == "none",
                "{name}: {mutation}"
            );
        }
    }
}

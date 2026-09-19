use nx::parse_declared_packages;
use std::collections::BTreeSet;

fn versions(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn preserves_package_identity_versions_and_duplicates() {
    let before = parse_declared_packages(include_str!("fixtures/packages-before.json")).unwrap();

    assert_eq!(before["alpha"], versions(&["1.0"]));
    assert_eq!(before["multi"], versions(&["1.0", "2.0"]));
    assert_eq!(before["unknown"], versions(&["<unknown>"]));
}

#[test]
fn before_and_after_fixtures_express_expected_changes() {
    let before = parse_declared_packages(include_str!("fixtures/packages-before.json")).unwrap();
    let after = parse_declared_packages(include_str!("fixtures/packages-after.json")).unwrap();

    assert_eq!(before["alpha"], versions(&["1.0"]));
    assert_eq!(after["alpha"], versions(&["2.0"]));
    assert_eq!(before["multi"], after["multi"]);
    assert!(!before.contains_key("added"));
    assert!(!after.contains_key("removed"));
}

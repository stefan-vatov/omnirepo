//! Focused proof for partial-file topology scanning.

#![allow(dead_code, unused_imports)]

use super::{Bounds, Topology, scan_partial};
use crate::managed_content::delimiters::{DelimiterSyntax, lookup};

fn yaml() -> &'static DelimiterSyntax {
    lookup("yaml").expect("yaml")
}

#[test]
fn exactly_one_ordered_non_nested_pair_yields_bounds() {
    let content = "a: 1\n# omnirepo-start\nmanaged: true\n# omnirepo-end\nb: 2\n";
    let topology = scan_partial(content, yaml());
    assert_eq!(
        topology,
        Topology::ExactlyOne {
            bounds: Bounds {
                start_line: 2,
                end_line: 4
            }
        }
    );
}

#[test]
fn absent_is_distinct_from_ambiguous() {
    assert_eq!(scan_partial("no markers here\n", yaml()), Topology::Absent);
    assert!(
        matches!(
            scan_partial("# omnirepo-start\nno close\n", yaml()),
            Topology::Ambiguous { .. }
        ),
        "unpaired open"
    );
    assert!(
        matches!(
            scan_partial("# omnirepo-end\nno open\n", yaml()),
            Topology::Ambiguous { .. }
        ),
        "unpaired close"
    );
}

#[test]
fn every_ambiguous_topology_returns_contextual_failure() {
    // Reversed order.
    assert!(matches!(
        scan_partial("# omnirepo-end\n# omnirepo-start\n", yaml()),
        Topology::Ambiguous { reason } if reason.contains("does not precede")
    ));
    // Multiple pairs.
    assert!(matches!(
        scan_partial(
            "# omnirepo-start\nx\n# omnirepo-end\n# omnirepo-start\ny\n# omnirepo-end\n",
            yaml()
        ),
        Topology::Ambiguous { reason } if reason.contains("more than one")
    ));
}

#[test]
fn original_content_remains_untouched() {
    let content = "a: 1\n# omnirepo-start\nmanaged: true\n# omnirepo-end\n";
    let snapshot = content.to_owned();
    let _ = scan_partial(content, yaml());
    assert_eq!(content, snapshot, "scanning never mutates the content");
}

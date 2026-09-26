//! Keystroke search (Section 4): every prefix of a query, against the
//! large fixture, must return quickly enough to run on each keystroke.

mod common;

fn main() {
    let (store, fx) = common::populated("search");
    let space = fx.space_id;
    let queries = [
        "call mom",
        "renew passport",
        "quarterly report",
        "gym",
        "xylophone",
    ];

    let mut warm = common::Stats::new();
    for q in queries {
        warm.time(|| store.search(space, q, 50).unwrap());
    }

    let mut per_keystroke = common::Stats::new();
    let mut first_char = common::Stats::new();
    let mut hits_total = 0usize;
    for _ in 0..20 {
        for q in queries {
            for end in 1..=q.len() {
                let prefix = &q[..end];
                let hits = if end == 1 {
                    first_char.time(|| store.search(space, prefix, 50).unwrap())
                } else {
                    per_keystroke.time(|| store.search(space, prefix, 50).unwrap())
                };
                hits_total += hits.len();
            }
        }
    }
    first_char.report("search, single-character prefix");
    per_keystroke.report("search, later keystrokes");
    println!("total hits across runs: {hits_total}");

    // Listing a list's tasks in manual order, as the main view does.
    let mut listing = common::Stats::new();
    for list in fx.lists.iter().take(10) {
        listing.time(|| {
            store
                .tasks(
                    space,
                    &liste_core::store::TaskFilter {
                        list: Some(Some(*list)),
                        ..Default::default()
                    },
                )
                .unwrap()
        });
    }
    listing.report("list view (one list, all live tasks)");
}

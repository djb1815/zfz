use zfz::frecency::Record;

use crate::{DirectoryRecord, State};

const COMPONENTS: &[&str] = &[
    "projects",
    "Documents",
    "src",
    "client-work",
    "archive",
    "api",
    "services",
    "research",
    "notes",
    "builds",
    "examples",
    "infra",
    "personal",
    "vendor",
];

/// Produces deterministic datasets without retaining visit logs.
#[must_use]
pub fn generate(count: usize) -> State {
    let tick = count as u64 * 8 + 100;
    let mut records = Vec::with_capacity(count);
    for index in 0..count {
        let a = COMPONENTS[index % COMPONENTS.len()];
        let b = COMPONENTS[(index.wrapping_mul(7) + 3) % COMPONENTS.len()];
        let suffix = match index % 97 {
            0 => format!("space dir/{index:06}"),
            1 => format!("naïve-café/{index:06}"),
            2 => format!("punctuation_[x]/{index:06}"),
            3 => format!("tab\tcomponent/{index:06}"),
            4 => format!("line\nbreak/{index:06}"),
            5 => format!("link/../spelling/{index:06}"),
            _ => format!("repo-{index:06}/worktree"),
        };
        let path = format!("/Users/benchmark/{a}/{b}/{suffix}");
        let visits = 1 + (index.wrapping_mul(31) % 250) as u64;
        let last_tick = tick - (index.wrapping_mul(17) % count.max(1)) as u64;
        let score = 1.0 + (index.wrapping_mul(13) % 120) as f64 / 10.0;
        records.push(DirectoryRecord {
            path,
            history: Record {
                visits,
                last_tick,
                score,
            },
        });
    }
    records.sort_by(|left, right| left.path.cmp(&right.path));
    records.dedup_by(|left, right| left.path == right.path);
    State { tick, records }
}

#[cfg(test)]
mod tests {
    use super::generate;

    #[test]
    fn generated_state_is_deterministic_valid_and_exactly_sized() {
        let state = generate(1_000);
        assert_eq!(state.records.len(), 1_000);
        assert_eq!(state, generate(1_000));
        state.validate().unwrap();
        assert!(state.records.iter().any(|record| record.path.contains(' ')));
        assert!(state.records.iter().any(|record| record.path.contains('ï')));
        assert!(
            state
                .records
                .iter()
                .any(|record| record.path.contains('\n'))
        );
    }
}

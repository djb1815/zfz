# Frecency prototype

## Source model

zfz's initial frecency prototype adopts the event-clock exponential moving sum
(EMS) model used by [ze](https://github.com/jghub/ze), as inspected on
2026-09-05 (upstream reports version `v3.3.2+`). It is not the wall-clock
scoring model used by classic `z`.

Every tracked directory visit advances one global event-clock tick. For a
directory with stored score `s` at its previous visit tick `t`, recording a
visit at the next global tick `T` stores:

```text
score = s * exp(-lambda * (T - t)) + 1
visits = visits + 1
last_tick = T
```

At query time, with global clock `N`, its current frecency score is:

```text
score_at_query = score * exp(-lambda * (N - last_tick))
```

The default decay constant is `lambda = 0.008` per directory-change event.
Its effective half-life is `ln(2) / lambda`, or approximately 86.64 directory
changes. Idle wall-clock time does not affect scores.

## Required incremental state

Each directory needs only the following state; the persistence layer must also
retain a global event-clock value (the greatest `last_tick`).

| Field | Purpose |
| --- | --- |
| `visits` | Frequency-only (`--rank`) ordering |
| `last_tick` | Recency-only (`--time`) ordering and score decay |
| `score` | EMS value at `last_tick` |

Default ranking orders by the decayed query-time score. Frequency-only ranking
orders by `visits`; recency-only ranking orders by `last_tick`. The prototype
does not prescribe final tie-breaking or persistence encoding.

## Upstream acknowledgement

[ze](https://github.com/jghub/ze) is MIT licensed and provides the documented
EMS model that this prototype evaluates. zfz implements that model independently
in Rust.

//! Filter → SQL. The **only** place that knows how an [`ExportFilter`] becomes a
//! `WHERE` clause.
//!
//! Two rules hold everywhere in this file:
//!
//! 1. **Every user value is a bound parameter.** Nothing a human typed is ever
//!    formatted into the SQL string. The single exception is a service name
//!    inside a `json_extract` path (SQLite takes no parameter in a path
//!    literal), and those are whitelisted to `[A-Za-z0-9_]` by
//!    [`ExportFilter::validate`] before they get here.
//! 2. **Categories AND, values within a category OR, `not_*` negates.**
//!
//! Filters over fields that aren't columns (continent, zones, contest id, the
//! `MY_*` snapshot, QSL status) read the `extra` JSON via [`json_field`]. Those
//! are unindexed — fine for a file export, and why the UI debounces its live
//! count.

use rusqlite::types::Value;

use super::filter::ExportFilter;
use crate::normalize;

/// The 14-char sortable `YYYYMMDDHHMMSS` key of a row. `time_on` is padded
/// because ADIF permits a 4-digit `HHMM`, which would otherwise sort wrong
/// against a 6-digit one.
pub(crate) const TS_EXPR: &str = "(qso_date || substr(time_on || '000000', 1, 6))";

/// A SQL expression reading one ADIF field out of the `extra` JSON blob.
///
/// `field` is always one of our own constants or a validated service-qualified
/// name — never raw user text. Kept in one place so these can be promoted to
/// real (indexed) columns later without touching any call site.
fn json_field(field: &str) -> String {
    format!("json_extract(extra, '$.{field}')")
}

/// A `WHERE` clause and the parameters to bind to it, in order.
pub struct BuiltQuery {
    /// Never empty — `"1=1"` when the filter is unconstrained, so callers can
    /// always splice it in.
    pub where_sql: String,
    pub params: Vec<Value>,
}

#[derive(Default)]
struct Builder {
    clauses: Vec<String>,
    params: Vec<Value>,
}

impl Builder {
    /// Add a clause with its bound parameters.
    fn push(&mut self, clause: impl Into<String>, params: impl IntoIterator<Item = Value>) {
        self.clauses.push(clause.into());
        self.params.extend(params);
    }

    /// `expr IN (?,?,?)` — the within-a-category OR.
    fn push_in(&mut self, expr: &str, values: impl IntoIterator<Item = Value>) {
        let values: Vec<Value> = values.into_iter().collect();
        if values.is_empty() {
            return; // validate() already rejects this; belt and braces.
        }
        let holes = std::iter::repeat_n("?", values.len())
            .collect::<Vec<_>>()
            .join(",");
        self.push(format!("{expr} IN ({holes})"), values);
    }

    fn build(self) -> BuiltQuery {
        let where_sql = if self.clauses.is_empty() {
            "1=1".to_string()
        } else {
            self.clauses.join(" AND ")
        };
        BuiltQuery {
            where_sql,
            params: self.params,
        }
    }
}

fn text(s: &str) -> Value {
    Value::Text(s.trim().to_string())
}
fn text_uc(s: &str) -> Value {
    Value::Text(s.trim().to_ascii_uppercase())
}
fn int(i: i64) -> Value {
    Value::Integer(i)
}

/// Escape the SQL `LIKE` metacharacters in a literal, so a user's `%` or `_`
/// matches itself. Pairs with `ESCAPE '\'`.
fn escape_like_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\\' || c == '%' || c == '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Translate a user wildcard (`*` = any run, `?` = any one char) into a SQL
/// `LIKE` pattern, escaping everything else. The result is **bound**, never
/// interpolated — so an input like `'; DROP TABLE qso;--` is just a (fruitless)
/// literal pattern.
pub(crate) fn wildcard_to_like(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    for c in pattern.trim().chars() {
        match c {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// The `extra` field name a QSL-status filter reads:
/// unscoped → `QSL_SENT`; scoped to a service → `LOTW_QSL_SENT`.
fn qsl_field(service: Option<&str>, suffix: &str) -> String {
    match service {
        Some(svc) => format!("{}_QSL_{suffix}", svc.trim().to_ascii_uppercase()),
        None => format!("QSL_{suffix}"),
    }
}

/// An `EXISTS` / `NOT EXISTS` correlated subquery against `qso_service`.
///
/// `qso_service` is empty until a later phase populates it, which makes these
/// correctly inert rather than broken: `NOT EXISTS` matches every QSO, `EXISTS`
/// matches none. They start working on their own when the table fills.
fn service_exists(negate: bool, when: &str, n: usize) -> String {
    let holes = std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",");
    let not = if negate { "NOT " } else { "" };
    format!(
        "{not}EXISTS (SELECT 1 FROM qso_service s \
         WHERE s.qso_id = qso.id AND s.service IN ({holes}) AND s.{when} IS NOT NULL)"
    )
}

/// Translate a validated filter into `WHERE` + params.
///
/// `bookmark` is the resolved `since_last_export` position — the caller reads it
/// from `export_state` and passes it in, so this stays a pure function.
/// `None` (no bookmark row yet) means "everything", not "nothing": the first
/// incremental export of a fresh profile exports the whole log.
pub fn build(filter: &ExportFilter, bookmark: Option<i64>) -> BuiltQuery {
    let mut b = Builder::default();

    // ---- A. date / time ----
    if let Some(d) = &filter.date_from {
        b.push("qso_date >= ?", [text(d)]);
    }
    if let Some(d) = &filter.date_to {
        b.push("qso_date <= ?", [text(d)]);
    }
    if let Some(ts) = &filter.datetime_from {
        b.push(format!("{TS_EXPR} >= ?"), [Value::Text(ts.key())]);
    }
    if let Some(ts) = &filter.datetime_to {
        b.push(format!("{TS_EXPR} <= ?"), [Value::Text(ts.key())]);
    }
    if filter.since_last_export.is_some()
        && let Some(last_id) = bookmark
    {
        // Insertion order, not contact time: this is export progress.
        b.push("id > ?", [int(last_id)]);
    }

    // ---- B. frequency / band / mode ----
    if let Some(bands) = &filter.bands {
        let holes = std::iter::repeat_n("?", bands.len())
            .collect::<Vec<_>>()
            .join(",");
        if filter.match_either_band {
            // The rare split-across-bands QSO: match if either side is in the set.
            b.push(
                format!(
                    "(band COLLATE NOCASE IN ({holes}) OR band_rx COLLATE NOCASE IN ({holes}))"
                ),
                bands
                    .iter()
                    .map(|s| text(s))
                    .chain(bands.iter().map(|s| text(s))),
            );
        } else {
            b.push_in("band COLLATE NOCASE", bands.iter().map(|s| text(s)));
        }
    }
    if let Some(f) = filter.freq_from {
        b.push("freq_hz >= ?", [int(f as i64)]);
    }
    if let Some(f) = filter.freq_to {
        b.push("freq_hz <= ?", [int(f as i64)]);
    }
    if let Some(modes) = &filter.modes {
        b.push_in("mode COLLATE NOCASE", modes.iter().map(|s| text_uc(s)));
    }
    if let Some(subs) = &filter.submodes {
        b.push_in("submode COLLATE NOCASE", subs.iter().map(|s| text_uc(s)));
    }
    if let Some(classes) = &filter.mode_classes {
        // Expanded through the normalizer's table, never hardcoded here.
        let modes: Vec<Value> = classes
            .iter()
            .flat_map(|c| normalize::modes_in_class(*c))
            .map(|m| Value::Text(m.to_string()))
            .collect();
        b.push_in("mode COLLATE NOCASE", modes);
    }

    // ---- C. worked station / entity ----
    if let Some(c) = &filter.call_exact {
        b.push("call = ? COLLATE NOCASE", [text_uc(c)]);
    }
    if let Some(p) = &filter.call_pattern {
        b.push(
            r"call LIKE ? ESCAPE '\'",
            [Value::Text(wildcard_to_like(p))],
        );
    }
    if let Some(p) = &filter.call_prefix {
        b.push(
            r"call LIKE ? ESCAPE '\'",
            [Value::Text(format!("{}%", escape_like_literal(p.trim())))],
        );
    }
    if let Some(d) = &filter.dxcc {
        b.push_in("dxcc", d.iter().map(|v| int(*v)));
    }
    if let Some(c) = &filter.continent {
        b.push_in(
            &format!("upper({})", json_field("CONT")),
            c.iter().map(|s| text_uc(s)),
        );
    }
    if let Some(z) = &filter.cq_zone {
        // CAST both sides: a log may store "05" where the user filters 5.
        b.push_in(
            &format!("CAST({} AS INTEGER)", json_field("CQZ")),
            z.iter().map(|v| int(*v)),
        );
    }
    if let Some(z) = &filter.itu_zone {
        b.push_in(
            &format!("CAST({} AS INTEGER)", json_field("ITUZ")),
            z.iter().map(|v| int(*v)),
        );
    }
    if let Some(g) = &filter.gridsquare {
        push_grid(&mut b, "gridsquare", g, filter.grid_precision);
    }
    if let Some(s) = &filter.state {
        b.push_in(
            &format!("upper({})", json_field("STATE")),
            s.iter().map(|s| text_uc(s)),
        );
    }
    if let Some(c) = &filter.country {
        b.push_in(
            &format!("upper({})", json_field("COUNTRY")),
            c.iter().map(|s| text_uc(s)),
        );
    }

    // ---- D. my station ----
    //
    // These read the per-QSO `extra` snapshot, NOT the `station` table. The
    // station row is keyed on callsign and updated in place, so joining it would
    // return today's grid for every historical QSO — the exact failure the
    // snapshot exists to prevent. Filtering "QSOs I made from EM12" must still
    // find them after the operator moves to FN31.
    if let Some(ids) = &filter.station_ids {
        b.push_in("station_id", ids.iter().map(|v| int(*v)));
    }
    if let Some(g) = &filter.my_gridsquare {
        push_grid(
            &mut b,
            &json_field("MY_GRIDSQUARE"),
            g,
            filter.grid_precision,
        );
    }
    if let Some(op) = &filter.operator {
        b.push(
            format!("upper({}) = ?", json_field("OPERATOR")),
            [text_uc(op)],
        );
    }
    if let Some(sc) = &filter.station_callsign {
        b.push(
            format!("upper({}) = ?", json_field("STATION_CALLSIGN")),
            [text_uc(sc)],
        );
    }

    // ---- E. confirmation / service ----
    for (services, negate, when) in [
        (&filter.uploaded_to, false, "uploaded_at"),
        (&filter.not_uploaded_to, true, "uploaded_at"),
        (&filter.confirmed_by, false, "confirmed_at"),
        (&filter.not_confirmed_by, true, "confirmed_at"),
    ] {
        if let Some(svcs) = services {
            b.push(
                service_exists(negate, when, svcs.len()),
                svcs.iter().map(|s| Value::Text(s.trim().to_lowercase())),
            );
        }
    }
    for (q, suffix) in [(&filter.qsl_sent, "SENT"), (&filter.qsl_rcvd, "RCVD")] {
        if let Some(q) = q {
            let field = qsl_field(q.service.as_deref(), suffix);
            b.push_in(
                &format!("upper({})", json_field(&field)),
                q.statuses.iter().map(|s| text_uc(s)),
            );
        }
    }

    // ---- F. contest ----
    if let Some(ids) = &filter.contest_ids {
        b.push_in(
            &format!("upper({})", json_field("CONTEST_ID")),
            ids.iter().map(|s| text_uc(s)),
        );
    }

    // ---- G. explicit selection ----
    // ANDs with everything else: a selection you may further narrow.
    if let Some(ids) = &filter.qso_ids {
        b.push_in("id", ids.iter().map(|v| int(*v)));
    }

    b.build()
}

/// Compare a grid expression at the requested precision. `Field`/`Square` clip
/// both sides to 2/4 chars so `EM` matches `EM12ab`.
fn push_grid(b: &mut Builder, expr: &str, value: &str, precision: super::filter::GridPrecision) {
    match precision.chars() {
        Some(n) => b.push(
            format!("upper(substr({expr}, 1, {n})) = ?"),
            [Value::Text(
                value
                    .trim()
                    .to_ascii_uppercase()
                    .chars()
                    .take(n)
                    .collect::<String>(),
            )],
        ),
        None => b.push(format!("upper({expr}) = ?"), [text_uc(value)]),
    }
}

/// `ORDER BY` for the requested sort. Ties break on `id` so the output is
/// deterministic (two QSOs can share a date+time).
pub fn order_by(sort: super::filter::Sort) -> &'static str {
    match sort {
        super::filter::Sort::Chronological => "ORDER BY qso_date ASC, time_on ASC, id ASC",
        super::filter::Sort::Reverse => "ORDER BY qso_date DESC, time_on DESC, id DESC",
    }
}

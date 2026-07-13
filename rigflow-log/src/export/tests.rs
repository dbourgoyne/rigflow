//! Export tests.
//!
//! These run against **on-disk** databases (not `open_in_memory`), because the
//! read-only-connection invariant is only real against a file — an in-memory DB
//! would let a bug in `Exporter::open` pass unnoticed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::filter::*;
use super::writer::*;
use crate::adif;
use crate::model::{Qso, Station};
use crate::normalize::ModeClass;
use crate::store::LogStore;

// ---------------------------------------------------------------- fixtures --

static SEQ: AtomicU32 = AtomicU32::new(0);

/// A fresh temp dir, unique per test (no `tempfile` dev-dep in this crate).
struct TmpDir(PathBuf);

impl TmpDir {
    fn new() -> TmpDir {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("rigflow-log-export-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        TmpDir(dir)
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn db(&self) -> PathBuf {
        self.join("rigflow_log.db")
    }
    fn adi(&self) -> PathBuf {
        self.join("rigflow_log.adi")
    }
    fn out(&self) -> PathBuf {
        self.join("out.adi")
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn station() -> Station {
    Station {
        station_call: "N0CALL".into(),
        gridsquare: Some("EM12".into()),
        name: Some("Dave".into()),
        ..Default::default()
    }
}

/// A QSO builder that starts from a sane 20m SSB contact.
fn qso(call: &str) -> Qso {
    Qso {
        call: call.into(),
        qso_date: "20260711".into(),
        time_on: "140000".into(),
        band: "20m".into(),
        mode: "SSB".into(),
        freq_hz: Some(14_207_000),
        rst_sent: Some("59".into()),
        rst_rcvd: Some("59".into()),
        ..Default::default()
    }
}

fn with_extra(mut q: Qso, kvs: &[(&str, &str)]) -> Qso {
    for (k, v) in kvs {
        q.extra.insert((*k).to_string(), (*v).to_string());
    }
    q
}

/// Open a store, insert `qsos`, return the store (still open, for bookmarks).
fn store_with(dir: &TmpDir, qsos: Vec<Qso>) -> LogStore {
    let mut s = LogStore::open(dir.db(), dir.adi()).unwrap();
    for q in qsos {
        s.insert(&q, &station()).unwrap();
    }
    s
}

/// The default fleet: 20m SSB, 40m CW, 20m FT8, 15m SSB.
fn fleet() -> Vec<Qso> {
    let mut ssb20 = qso("W1AW");
    ssb20.qso_date = "20260701".into();

    let mut cw40 = qso("K5ZD");
    cw40.qso_date = "20260702".into();
    cw40.band = "40m".into();
    cw40.mode = "CW".into();
    cw40.freq_hz = Some(7_030_000);

    let mut ft8 = qso("JA1XYZ");
    ft8.qso_date = "20260703".into();
    ft8.mode = "FT8".into();
    ft8.freq_hz = Some(14_074_000);

    let mut ssb15 = qso("VK3ABC");
    ssb15.qso_date = "20260704".into();
    ssb15.band = "15m".into();
    ssb15.freq_hz = Some(21_300_000);

    vec![ssb20, cw40, ft8, ssb15]
}

/// Export with a filter and return the calls in the written file, in file order.
fn exported_calls(dir: &TmpDir, filter: &ExportFilter) -> Vec<String> {
    let opts = ExportOptions {
        output_path: dir.out(),
        ..Default::default()
    };
    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(filter, &opts).unwrap();
    read_calls(&dir.out())
}

fn read_calls(path: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    adif::parse_adif(&text)
        .unwrap()
        .iter()
        .map(|r| r.get("CALL").cloned().unwrap_or_default())
        .collect()
}

fn count(dir: &TmpDir, filter: &ExportFilter) -> usize {
    Exporter::open(dir.db()).unwrap().count(filter).unwrap()
}

// ------------------------------------------------------------ each filter --

#[test]
fn filter_by_band() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let f = ExportFilter {
        bands: Some(vec!["20m".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW", "JA1XYZ"]);
}

#[test]
fn filter_multi_value_within_a_category_ors() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let f = ExportFilter {
        bands: Some(vec!["40m".into(), "15m".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["K5ZD", "VK3ABC"]);
}

#[test]
fn filter_categories_and() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    // 20m AND SSB → only W1AW (JA1XYZ is 20m but FT8).
    let f = ExportFilter {
        bands: Some(vec!["20m".into()]),
        modes: Some(vec!["SSB".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW"]);
}

#[test]
fn filter_by_mode_class_expands_through_the_normalizer() {
    let dir = TmpDir::new();
    let mut fleet = fleet();
    // An FT4 QSO stores as MODE=MFSK / SUBMODE=FT4. A "digital" class filter has
    // to find it via MFSK, which is exactly what the shared class table is for.
    let mut ft4 = qso("EA1AAA");
    ft4.qso_date = "20260705".into();
    ft4.mode = "FT4".into();
    ft4.normalize();
    assert_eq!(ft4.mode, "MFSK");
    fleet.push(ft4);
    let _s = store_with(&dir, fleet);

    let f = ExportFilter {
        mode_classes: Some(vec![ModeClass::Digital]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["JA1XYZ", "EA1AAA"]);

    let f = ExportFilter {
        mode_classes: Some(vec![ModeClass::Phone, ModeClass::Cw]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW", "K5ZD", "VK3ABC"]);
}

#[test]
fn filter_by_date_range_inclusive() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let f = ExportFilter {
        date_from: Some("20260702".into()),
        date_to: Some("20260703".into()),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["K5ZD", "JA1XYZ"]);
}

#[test]
fn filter_by_datetime_range() {
    let dir = TmpDir::new();
    let mut a = qso("AAA");
    a.qso_date = "20260701".into();
    a.time_on = "115900".into();
    let mut b = qso("BBB");
    b.qso_date = "20260701".into();
    b.time_on = "120100".into();
    let _s = store_with(&dir, vec![a, b]);

    let f = ExportFilter {
        datetime_from: Some(Timestamp {
            date: "20260701".into(),
            time: "120000".into(),
        }),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["BBB"]);
}

#[test]
fn filter_by_freq_range() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let f = ExportFilter {
        freq_from: Some(14_000_000),
        freq_to: Some(14_100_000),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["JA1XYZ"]);
}

#[test]
fn filter_by_call_exact_and_prefix() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());

    let f = ExportFilter {
        call_exact: Some("k5zd".into()), // case-insensitive
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["K5ZD"]);

    let f = ExportFilter {
        call_prefix: Some("JA".into()),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["JA1XYZ"]);
}

#[test]
fn filter_by_call_pattern_wildcards() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());

    let f = ExportFilter {
        call_pattern: Some("*1*".into()),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW", "JA1XYZ"]);

    // '?' is exactly one char: "K?ZD" matches K5ZD.
    let f = ExportFilter {
        call_pattern: Some("K?ZD".into()),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["K5ZD"]);
}

#[test]
fn filter_by_dxcc_and_extra_backed_entity_fields() {
    let dir = TmpDir::new();
    let qsos = vec![
        with_extra(
            qso("JA1XYZ"),
            &[("CONT", "AS"), ("CQZ", "25"), ("STATE", "13")],
        ),
        with_extra(
            qso("W1AW"),
            &[("CONT", "NA"), ("CQZ", "05"), ("STATE", "CT")],
        ),
    ];
    let mut qsos = qsos;
    qsos[0].dxcc = Some(339);
    qsos[1].dxcc = Some(291);
    let _s = store_with(&dir, qsos);

    let f = ExportFilter {
        dxcc: Some(vec![339]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["JA1XYZ"]);

    let f = ExportFilter {
        continent: Some(vec!["NA".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW"]);

    // Zones compare numerically: stored "05" must match a filter of 5.
    let f = ExportFilter {
        cq_zone: Some(vec![5]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW"]);

    let f = ExportFilter {
        state: Some(vec!["CT".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW"]);
}

#[test]
fn filter_by_gridsquare_precision() {
    let dir = TmpDir::new();
    let mut a = qso("AAA");
    a.gridsquare = Some("EM12ab".into());
    let mut b = qso("BBB");
    b.gridsquare = Some("FN31pr".into());
    let _s = store_with(&dir, vec![a, b]);

    // Field (2 chars).
    let f = ExportFilter {
        gridsquare: Some("EM".into()),
        grid_precision: GridPrecision::Field,
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["AAA"]);

    // Square (4 chars).
    let f = ExportFilter {
        gridsquare: Some("FN31".into()),
        grid_precision: GridPrecision::Square,
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["BBB"]);

    // Full: exact.
    let f = ExportFilter {
        gridsquare: Some("EM12AB".into()),
        grid_precision: GridPrecision::Full,
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["AAA"]);
}

#[test]
fn filter_by_contest_id_from_extra_json() {
    let dir = TmpDir::new();
    let _s = store_with(
        &dir,
        vec![
            with_extra(qso("AAA"), &[("CONTEST_ID", "CQ-WW-SSB")]),
            qso("BBB"),
        ],
    );
    let f = ExportFilter {
        contest_ids: Some(vec!["CQ-WW-SSB".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["AAA"]);
}

#[test]
fn filter_by_explicit_qso_ids_and_it_ands() {
    let dir = TmpDir::new();
    let mut s = LogStore::open(dir.db(), dir.adi()).unwrap();
    let ids: Vec<i64> = fleet()
        .into_iter()
        .map(|q| s.insert(&q, &station()).unwrap().id)
        .collect();
    drop(s);

    // Selection alone.
    let f = ExportFilter {
        qso_ids: Some(vec![ids[0], ids[2]]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW", "JA1XYZ"]);

    // Selection AND another filter narrows it (documented behavior).
    let f = ExportFilter {
        qso_ids: Some(vec![ids[0], ids[2]]),
        modes: Some(vec!["FT8".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["JA1XYZ"]);
}

// ------------------------------------------------- the view IS the export --

#[test]
fn the_page_the_view_shows_is_the_set_the_export_writes() {
    // The contact view and the export share one WHERE builder, so for any filter
    // the listed calls and the exported calls must be the same set. This is the
    // property the shared-filter UI promises the operator ("you see what you are
    // exporting"); if it ever breaks, it breaks silently, so pin it.
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let ex = Exporter::open(dir.db()).unwrap();

    let filters = [
        ExportFilter::default(),
        ExportFilter {
            bands: Some(vec!["20m".into()]),
            ..Default::default()
        },
        ExportFilter {
            modes: Some(vec!["SSB".into()]),
            ..Default::default()
        },
        ExportFilter {
            call_pattern: Some("*1*".into()),
            ..Default::default()
        },
        ExportFilter {
            call_exact: Some("NOBODY".into()), // empty set
            ..Default::default()
        },
    ];

    for f in filters {
        let page = ex.page(&f, 500, Sort::Reverse).unwrap();
        let exported = exported_calls(&dir, &f);

        assert_eq!(
            page.total,
            exported.len(),
            "view total disagrees with what the export wrote, filter {f:?}"
        );

        let mut listed: Vec<String> = page.rows.iter().map(|r| r.qso.call.clone()).collect();
        let mut written = exported.clone();
        listed.sort();
        written.sort();
        assert_eq!(listed, written, "view rows != exported rows, filter {f:?}");
    }
}

#[test]
fn the_page_total_is_honest_about_the_row_cap() {
    // The view caps its rows; the export does not. So `total` must report the
    // real match count, not the number of rows returned — otherwise a capped view
    // silently implies it is showing everything the export will write.
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet()); // 4 QSOs
    let ex = Exporter::open(dir.db()).unwrap();

    let page = ex.page(&ExportFilter::default(), 2, Sort::Reverse).unwrap();
    assert_eq!(page.rows.len(), 2, "capped by limit");
    assert_eq!(page.total, 4, "but the total is the whole match set");
}

#[test]
fn view_page_is_newest_first() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let ex = Exporter::open(dir.db()).unwrap();
    let page = ex
        .page(&ExportFilter::default(), 500, Sort::Reverse)
        .unwrap();
    let calls: Vec<&str> = page.rows.iter().map(|r| r.qso.call.as_str()).collect();
    assert_eq!(calls, ["VK3ABC", "JA1XYZ", "K5ZD", "W1AW"]);
}

#[test]
fn call_lookup_searches_the_whole_log_ignoring_any_view_filter() {
    // The call-lookup popup answers "have I EVER worked this station?", so it is
    // built from a filter of its own (call_exact alone) rather than from the view
    // filter. A 20m-only view must not hide the 40m QSO with the same station.
    let dir = TmpDir::new();
    let mut a = qso("DX1ABC");
    a.qso_date = "20260701".into();
    let mut b = qso("DX1ABC");
    b.qso_date = "20260702".into();
    b.band = "40m".into();
    b.mode = "CW".into();
    b.freq_hz = Some(7_030_000);
    let _s = store_with(&dir, vec![a, b, qso("OTHER")]);

    let lookup = ExportFilter {
        call_exact: Some("dx1abc".into()),
        ..Default::default()
    };
    let ex = Exporter::open(dir.db()).unwrap();
    let page = ex.page(&lookup, 100, Sort::Reverse).unwrap();

    assert_eq!(page.total, 2, "both bands, regardless of any view filter");
    let bands: Vec<&str> = page.rows.iter().map(|r| r.qso.band.as_str()).collect();
    assert_eq!(bands, ["40m", "20m"]);
}

#[test]
fn store_filtered_query_matches_the_exporter() {
    // The read-write store and the read-only exporter must agree — they are two
    // connections onto one builder.
    let dir = TmpDir::new();
    let s = store_with(&dir, fleet());
    let f = ExportFilter {
        bands: Some(vec!["20m".into()]),
        ..Default::default()
    };
    let via_store: Vec<String> = s
        .query_contacts_filtered(&f, 500)
        .unwrap()
        .iter()
        .map(|r| r.qso.call.clone())
        .collect();
    let via_exporter: Vec<String> = Exporter::open(dir.db())
        .unwrap()
        .page(&f, 500, Sort::Reverse)
        .unwrap()
        .rows
        .iter()
        .map(|r| r.qso.call.clone())
        .collect();
    assert_eq!(via_store, via_exporter);
}

// --------------------------------------------------------- my-station (D) --

#[test]
fn my_gridsquare_filters_the_snapshot_not_the_current_station_row() {
    // THE regression test for the station-table trap. `upsert_station` keys on
    // callsign and updates the row IN PLACE, so after an operator moves QTH the
    // station row holds the NEW grid for every historical QSO. Filtering
    // "QSOs I made from EM12" must still find the ones made before the move —
    // which only works because we filter the per-QSO MY_GRIDSQUARE snapshot.
    let dir = TmpDir::new();
    let mut s = LogStore::open(dir.db(), dir.adi()).unwrap();

    let from_em12 = Station {
        gridsquare: Some("EM12".into()),
        ..station()
    };
    s.insert(&qso("OLDQTH"), &from_em12).unwrap();

    // Operator moves. Same callsign → the SAME station row is rewritten.
    let from_fn31 = Station {
        gridsquare: Some("FN31".into()),
        ..station()
    };
    s.insert(&qso("NEWQTH"), &from_fn31).unwrap();

    let n: i64 = s
        .conn()
        .query_row("SELECT COUNT(*) FROM station", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        n, 1,
        "same callsign must reuse (and mutate) one station row"
    );
    let current: String = s
        .conn()
        .query_row("SELECT gridsquare FROM station", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        current, "FN31",
        "the station row now holds only the new grid"
    );
    drop(s);

    // The old QSO is still findable by the grid it was actually made from.
    let f = ExportFilter {
        my_gridsquare: Some("EM12".into()),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["OLDQTH"]);

    let f = ExportFilter {
        my_gridsquare: Some("FN31".into()),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["NEWQTH"]);
}

#[test]
fn filter_by_operator_and_station_callsign_snapshot() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let f = ExportFilter {
        operator: Some("n0call".into()),
        ..Default::default()
    };
    assert_eq!(count(&dir, &f), 4);

    let f = ExportFilter {
        station_callsign: Some("W9XYZ".into()),
        ..Default::default()
    };
    assert_eq!(count(&dir, &f), 0);
}

// ------------------------------------------------------- services (E) ------

#[test]
fn service_filters_are_inert_against_an_empty_qso_service() {
    // qso_service stays empty until a later phase populates it. That must make
    // these filters CORRECT-but-inert, not broken.
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());

    // Nothing has been uploaded, so "not uploaded to LoTW" is everything.
    let f = ExportFilter {
        not_uploaded_to: Some(vec!["lotw".into()]),
        ..Default::default()
    };
    assert_eq!(count(&dir, &f), 4);

    // ...and "uploaded to LoTW" / "confirmed by LoTW" are nothing.
    let f = ExportFilter {
        uploaded_to: Some(vec!["lotw".into()]),
        ..Default::default()
    };
    assert_eq!(count(&dir, &f), 0);

    let f = ExportFilter {
        confirmed_by: Some(vec!["lotw".into()]),
        ..Default::default()
    };
    assert_eq!(count(&dir, &f), 0);

    let f = ExportFilter {
        not_confirmed_by: Some(vec!["lotw".into()]),
        ..Default::default()
    };
    assert_eq!(count(&dir, &f), 4);
}

#[test]
fn service_filters_light_up_when_the_table_fills() {
    // Same filters, now with a populated qso_service — no code change, they just
    // start selecting. This is what "the seam is real" means.
    let dir = TmpDir::new();
    let mut s = LogStore::open(dir.db(), dir.adi()).unwrap();
    let ids: Vec<i64> = fleet()
        .into_iter()
        .map(|q| s.insert(&q, &station()).unwrap().id)
        .collect();
    s.conn()
        .execute(
            "INSERT INTO qso_service (qso_id, service, uploaded_at, confirmed_at) \
             VALUES (?1, 'lotw', '2026-07-12T00:00:00Z', NULL)",
            [ids[0]],
        )
        .unwrap();
    s.conn()
        .execute(
            "INSERT INTO qso_service (qso_id, service, uploaded_at, confirmed_at) \
             VALUES (?1, 'lotw', '2026-07-12T00:00:00Z', '2026-07-12T01:00:00Z')",
            [ids[1]],
        )
        .unwrap();
    drop(s);

    let f = ExportFilter {
        uploaded_to: Some(vec!["lotw".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W1AW", "K5ZD"]);

    let f = ExportFilter {
        confirmed_by: Some(vec!["lotw".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["K5ZD"]);

    // The complement: the two never uploaded.
    let f = ExportFilter {
        not_uploaded_to: Some(vec!["lotw".into()]),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["JA1XYZ", "VK3ABC"]);
}

#[test]
fn qsl_status_filters_read_extra_not_qso_service() {
    let dir = TmpDir::new();
    let _s = store_with(
        &dir,
        vec![
            with_extra(qso("AAA"), &[("QSL_RCVD", "Y")]),
            with_extra(qso("BBB"), &[("QSL_RCVD", "N")]),
            with_extra(qso("CCC"), &[("LOTW_QSL_RCVD", "Y")]),
        ],
    );

    // QSO-level.
    let f = ExportFilter {
        qsl_rcvd: Some(QslStatusFilter {
            service: None,
            statuses: vec!["Y".into()],
        }),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["AAA"]);

    // Service-qualified → LOTW_QSL_RCVD.
    let f = ExportFilter {
        qsl_rcvd: Some(QslStatusFilter {
            service: Some("lotw".into()),
            statuses: vec!["Y".into()],
        }),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["CCC"]);
}

// ---------------------------------------------------------- incremental ----

#[test]
fn incremental_export_advances_only_on_the_incremental_path() {
    let dir = TmpDir::new();
    let mut s = store_with(&dir, fleet());
    let profile = DEFAULT_EXPORT_PROFILE;

    assert_eq!(s.export_bookmark(profile).unwrap(), None);

    // --- an ad-hoc filtered export must NOT move the bookmark ---
    let adhoc = ExportFilter {
        bands: Some(vec!["20m".into()]),
        ..Default::default()
    };
    let ex = Exporter::open(dir.db()).unwrap();
    let opts = ExportOptions {
        output_path: dir.out(),
        ..Default::default()
    };
    let summary = ex.export(&adhoc, &opts).unwrap();
    assert_eq!(summary.count, 2);
    // The caller only advances on the incremental path — it isn't one, so it doesn't.
    assert_eq!(
        s.export_bookmark(profile).unwrap(),
        None,
        "an ad-hoc export must never move the incremental position"
    );

    // --- a dry run must NOT move it either ---
    let incremental = ExportFilter {
        since_last_export: Some(profile.to_string()),
        ..Default::default()
    };
    assert_eq!(ex.count(&incremental).unwrap(), 4, "first run: whole log");
    assert_eq!(s.export_bookmark(profile).unwrap(), None);

    // --- the real incremental export ---
    let summary = ex.export(&incremental, &opts).unwrap();
    assert_eq!(summary.count, 4);
    let max_id = summary.max_qso_id.unwrap();
    s.advance_export_bookmark(profile, max_id).unwrap();
    assert_eq!(s.export_bookmark(profile).unwrap(), Some(max_id));

    // --- a second incremental returns only what's newer ---
    assert_eq!(
        ex.count(&incremental).unwrap(),
        0,
        "nothing new since the bookmark"
    );

    let mut newer = qso("NEW1");
    newer.qso_date = "20260710".into();
    s.insert(&newer, &station()).unwrap();

    let ex = Exporter::open(dir.db()).unwrap();
    let summary = ex.export(&incremental, &opts).unwrap();
    assert_eq!(summary.count, 1);
    assert_eq!(read_calls(&dir.out()), ["NEW1"]);
}

#[test]
fn bookmarks_are_per_profile_and_never_rewind() {
    let dir = TmpDir::new();
    let mut s = store_with(&dir, fleet());

    s.advance_export_bookmark("wavelog", 3).unwrap();
    assert_eq!(s.export_bookmark("wavelog").unwrap(), Some(3));
    // A different stream is untouched — independent positions don't collide.
    assert_eq!(s.export_bookmark("club-log").unwrap(), None);

    // Re-exporting an older slice must not rewind the position.
    s.advance_export_bookmark("wavelog", 1).unwrap();
    assert_eq!(s.export_bookmark("wavelog").unwrap(), Some(3));
}

// ------------------------------------------------------- output mechanics --

#[test]
fn empty_match_set_writes_a_valid_zero_record_file() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let f = ExportFilter {
        call_exact: Some("NOBODY".into()),
        ..Default::default()
    };
    let ex = Exporter::open(dir.db()).unwrap();
    let summary = ex
        .export(
            &f,
            &ExportOptions {
                output_path: dir.out(),
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(summary.count, 0);
    let text = std::fs::read_to_string(dir.out()).unwrap();
    assert!(text.contains("<EOH>"), "header present");
    assert!(!text.contains("<EOR>"), "no records");
    // ...and it still parses as ADIF.
    assert!(adif::parse_adif(&text).unwrap().is_empty());
}

#[test]
fn dry_run_counts_without_writing_a_file() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let f = ExportFilter {
        bands: Some(vec!["20m".into()]),
        ..Default::default()
    };
    assert_eq!(count(&dir, &f), 2);
    assert!(
        !dir.out().exists(),
        "a dry run must not create the output file"
    );
}

#[test]
fn header_carries_version_program_and_created_timestamp() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());
    let ex = Exporter::open(dir.db()).unwrap();
    let summary = ex
        .export(
            &ExportFilter::default(),
            &ExportOptions {
                output_path: dir.out(),
                ..Default::default()
            },
        )
        .unwrap();

    let text = std::fs::read_to_string(dir.out()).unwrap();
    assert!(text.contains("<ADIF_VER:5>3.1.6"));
    assert!(text.contains("<PROGRAMID:7>rigflow"));
    assert!(text.contains(&format!(
        "<PROGRAMVERSION:{}>{}",
        PROGRAM_VERSION.len(),
        PROGRAM_VERSION
    )));
    assert!(text.contains(&format!(
        "<CREATED_TIMESTAMP:15>{}",
        summary.created_timestamp
    )));
    assert_eq!(summary.created_timestamp.len(), 15, "YYYYMMDD HHMMSS");
}

#[test]
fn sort_order() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());

    let chrono = ExportOptions {
        output_path: dir.out(),
        sort: Sort::Chronological,
        ..Default::default()
    };
    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(&ExportFilter::default(), &chrono).unwrap();
    assert_eq!(read_calls(&dir.out()), ["W1AW", "K5ZD", "JA1XYZ", "VK3ABC"]);

    let rev = ExportOptions {
        sort: Sort::Reverse,
        ..chrono
    };
    ex.export(&ExportFilter::default(), &rev).unwrap();
    assert_eq!(read_calls(&dir.out()), ["VK3ABC", "JA1XYZ", "K5ZD", "W1AW"]);
}

#[test]
fn split_fields_only_on_split_qsos() {
    let dir = TmpDir::new();
    let mut split = qso("DX1SPLIT");
    split.freq_rx_hz = Some(14_195_000);
    split.normalize();
    assert_eq!(split.band_rx.as_deref(), Some("20m"));
    let _s = store_with(&dir, vec![qso("SIMPLEX"), split]);

    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(
        &ExportFilter::default(),
        &ExportOptions {
            output_path: dir.out(),
            ..Default::default()
        },
    )
    .unwrap();

    let text = std::fs::read_to_string(dir.out()).unwrap();
    let records = adif::parse_adif(&text).unwrap();
    let simplex = records.iter().find(|r| r["CALL"] == "SIMPLEX").unwrap();
    let split_rec = records.iter().find(|r| r["CALL"] == "DX1SPLIT").unwrap();

    assert!(!simplex.contains_key("FREQ_RX"), "simplex omits FREQ_RX");
    assert!(!simplex.contains_key("BAND_RX"), "simplex omits BAND_RX");
    assert_eq!(split_rec.get("FREQ_RX").unwrap(), "14.195000");
    assert_eq!(split_rec.get("BAND_RX").unwrap(), "20m");
}

// --------------------------------------------------------- field profiles --

#[test]
fn field_profile_core_emits_only_the_core_subset() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, vec![with_extra(qso("AAA"), &[("MY_CITY", "Austin")])]);

    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(
        &ExportFilter::default(),
        &ExportOptions {
            output_path: dir.out(),
            field_profile: FieldProfile::Core,
            ..Default::default()
        },
    )
    .unwrap();

    let text = std::fs::read_to_string(dir.out()).unwrap();
    let rec = &adif::parse_adif(&text).unwrap()[0];
    for k in rec.keys() {
        assert!(
            CORE_FIELDS.contains(&k.as_str()),
            "Core emitted a non-core field: {k}"
        );
    }
    assert!(rec.contains_key("CALL") && rec.contains_key("BAND"));
    // The station snapshot and extras are not core.
    assert!(!rec.contains_key("MY_CITY"));
    assert!(!rec.contains_key("STATION_CALLSIGN"));
}

#[test]
fn field_profile_core_keeps_submode_so_ft4_survives() {
    // Core deliberately includes SUBMODE (the brief's list omitted it): without
    // it an FT4 QSO exports as bare MODE=MFSK and comes back as something else.
    let dir = TmpDir::new();
    let mut ft4 = qso("EA1AAA");
    ft4.mode = "FT4".into();
    ft4.normalize();
    let _s = store_with(&dir, vec![ft4]);

    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(
        &ExportFilter::default(),
        &ExportOptions {
            output_path: dir.out(),
            field_profile: FieldProfile::Core,
            ..Default::default()
        },
    )
    .unwrap();

    let text = std::fs::read_to_string(dir.out()).unwrap();
    let rec = &adif::parse_adif(&text).unwrap()[0];
    assert_eq!(rec.get("MODE").unwrap(), "MFSK");
    assert_eq!(rec.get("SUBMODE").unwrap(), "FT4");
}

#[test]
fn field_profile_custom_emits_exactly_the_requested_fields() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, vec![qso("AAA")]);

    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(
        &ExportFilter::default(),
        &ExportOptions {
            output_path: dir.out(),
            field_profile: FieldProfile::Custom(vec!["call".into(), "band".into()]),
            ..Default::default()
        },
    )
    .unwrap();

    let text = std::fs::read_to_string(dir.out()).unwrap();
    let rec = &adif::parse_adif(&text).unwrap()[0];
    let mut keys: Vec<&str> = rec.keys().map(|s| s.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["BAND", "CALL"]);
}

#[test]
fn include_extra_false_drops_the_passthrough() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, vec![with_extra(qso("AAA"), &[("APP_X", "1")])]);

    let ex = Exporter::open(dir.db()).unwrap();
    for (include, want) in [(true, true), (false, false)] {
        ex.export(
            &ExportFilter::default(),
            &ExportOptions {
                output_path: dir.out(),
                field_profile: FieldProfile::Full,
                include_extra: include,
                ..Default::default()
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.out()).unwrap();
        let rec = &adif::parse_adif(&text).unwrap()[0];
        assert_eq!(rec.contains_key("APP_X"), want);
        assert_eq!(rec.contains_key("STATION_CALLSIGN"), want);
        assert!(rec.contains_key("CALL"), "modeled columns always survive");
    }
}

// ------------------------------------------------------------- round-trip --

#[test]
fn round_trip_export_then_import_into_a_clean_db() {
    let src = TmpDir::new();
    let mut split = qso("DX1SPLIT");
    split.freq_rx_hz = Some(14_195_000);
    split.normalize();
    let qsos = vec![
        with_extra(qso("W1AW"), &[("COMMENT", "nice sig")]),
        split,
        with_extra(qso("OH2ÄÄ"), &[("NAME_INTL", "Jörg")]), // UTF-8 / _INTL
    ];
    let _s = store_with(&src, qsos.clone());

    let ex = Exporter::open(src.db()).unwrap();
    let summary = ex
        .export(
            &ExportFilter::default(),
            &ExportOptions {
                output_path: src.out(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(summary.count, 3);

    // Re-import into a clean DB via the shared parser.
    let dst = TmpDir::new();
    let text = std::fs::read_to_string(src.out()).unwrap();
    let imported = adif::parse_adif_to_qsos(&text).unwrap();
    let mut s2 = LogStore::open(dst.db(), dst.adi()).unwrap();
    for q in &imported {
        s2.insert(q, &station()).unwrap();
    }

    let round_tripped = s2.query_contacts(100).unwrap();
    assert_eq!(round_tripped.len(), 3);

    // Every source QSO survives, with its split fields and its extras.
    let by_call = |call: &str| {
        round_tripped
            .iter()
            .find(|lq| lq.qso.call == call)
            .unwrap()
            .qso
            .clone()
    };

    let split_back = by_call("DX1SPLIT");
    assert_eq!(split_back.freq_rx_hz, Some(14_195_000));
    assert_eq!(split_back.band_rx.as_deref(), Some("20m"));

    let w1aw = by_call("W1AW");
    assert_eq!(w1aw.freq_rx_hz, None, "simplex stays simplex");
    assert_eq!(w1aw.extra.get("COMMENT").unwrap(), "nice sig");

    let intl = by_call("OH2ÄÄ");
    assert_eq!(intl.extra.get("NAME_INTL").unwrap(), "Jörg");

    // The modeled columns match the source exactly.
    for src_q in &qsos {
        let back = by_call(&src_q.call);
        assert_eq!(back.band, src_q.band);
        assert_eq!(back.mode, src_q.mode);
        assert_eq!(back.freq_hz, src_q.freq_hz);
        assert_eq!(back.freq_rx_hz, src_q.freq_rx_hz);
        assert_eq!(back.rst_sent, src_q.rst_sent);
    }
}

// -------------------------------------------------------------- injection --

#[test]
fn call_pattern_is_parameterized_against_injection() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());

    let nasty = "'; DROP TABLE qso;--";
    let f = ExportFilter {
        call_pattern: Some(nasty.into()),
        ..Default::default()
    };
    // Matches nothing (no call looks like that) and, crucially, does no damage.
    assert_eq!(count(&dir, &f), 0);

    let s = LogStore::open(dir.db(), dir.adi()).unwrap();
    let n: i64 = s
        .conn()
        .query_row("SELECT COUNT(*) FROM qso", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 4, "the table is still there with all its rows");

    // A literal % in a call must match a literal %, not act as a wildcard.
    let mut odd = qso("W%AW");
    odd.qso_date = "20260709".into();
    let mut s = s;
    s.insert(&odd, &station()).unwrap();
    drop(s);

    let f = ExportFilter {
        call_exact: Some("W%AW".into()),
        ..Default::default()
    };
    assert_eq!(exported_calls(&dir, &f), ["W%AW"]);

    // '*' is the user's wildcard; a raw '%' is escaped to a literal.
    let f = ExportFilter {
        call_pattern: Some("W%*".into()),
        ..Default::default()
    };
    assert_eq!(
        exported_calls(&dir, &f),
        ["W%AW"],
        "the % is literal, the * is the wildcard"
    );
}

#[test]
fn service_name_with_sql_in_it_is_rejected_not_interpolated() {
    // A service name reaches SQL as text (inside a json_extract path), so it is
    // whitelisted at the door rather than bound.
    let f = ExportFilter {
        qsl_rcvd: Some(QslStatusFilter {
            service: Some("lotw'); DROP TABLE qso;--".into()),
            statuses: vec!["Y".into()],
        }),
        ..Default::default()
    };
    assert!(matches!(f.validate(), Err(FilterError::BadServiceName(_))));
}

// ------------------------------------------------------------ validation ---

#[test]
fn validation_rejects_bad_input_up_front() {
    let bad_date = ExportFilter {
        date_from: Some("2026-07-01".into()),
        ..Default::default()
    };
    assert!(matches!(bad_date.validate(), Err(FilterError::BadDate(_))));

    let not_a_day = ExportFilter {
        date_from: Some("20260231".into()), // Feb 31st
        ..Default::default()
    };
    assert!(matches!(not_a_day.validate(), Err(FilterError::BadDate(_))));

    let inverted = ExportFilter {
        date_from: Some("20260710".into()),
        date_to: Some("20260701".into()),
        ..Default::default()
    };
    assert!(matches!(
        inverted.validate(),
        Err(FilterError::InvertedRange { .. })
    ));

    let bad_band = ExportFilter {
        bands: Some(vec!["17.5m".into()]),
        ..Default::default()
    };
    assert!(matches!(
        bad_band.validate(),
        Err(FilterError::UnknownBand(_))
    ));

    let empty = ExportFilter {
        bands: Some(vec![]),
        ..Default::default()
    };
    assert!(matches!(empty.validate(), Err(FilterError::EmptyList(_))));

    let bad_cont = ExportFilter {
        continent: Some(vec!["XX".into()]),
        ..Default::default()
    };
    assert!(matches!(
        bad_cont.validate(),
        Err(FilterError::UnknownContinent(_))
    ));
}

#[test]
fn validation_rejects_non_canonical_modes_with_the_fix() {
    // USB is a sideband, not an ADIF mode: the column holds SSB, so filtering
    // USB would silently match zero of the operator's hundreds of SSB QSOs.
    let f = ExportFilter {
        modes: Some(vec!["USB".into()]),
        ..Default::default()
    };
    match f.validate() {
        Err(FilterError::NonCanonicalMode { given, canonical }) => {
            assert_eq!(given, "USB");
            assert_eq!(canonical, "SSB");
        }
        other => panic!("expected NonCanonicalMode, got {other:?}"),
    }

    // FT4 is stored as MFSK/FT4 — filter it as a submode (or by class).
    let f = ExportFilter {
        modes: Some(vec!["FT4".into()]),
        ..Default::default()
    };
    assert!(matches!(
        f.validate(),
        Err(FilterError::NonCanonicalMode { .. })
    ));

    // The canonical ones pass.
    let f = ExportFilter {
        modes: Some(vec!["SSB".into(), "CW".into(), "FT8".into(), "MFSK".into()]),
        ..Default::default()
    };
    assert!(f.validate().is_ok());
}

#[test]
fn custom_profile_with_no_fields_is_rejected() {
    let opts = ExportOptions {
        field_profile: FieldProfile::Custom(vec![]),
        ..Default::default()
    };
    assert!(matches!(opts.validate(), Err(FilterError::EmptyFieldList)));
}

// ----------------------------------------------------------- read-only -----

#[test]
fn the_exporter_connection_cannot_write() {
    // The read-only invariant is enforced by SQLite, not by our discipline.
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());

    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(
        &ExportFilter::default(),
        &ExportOptions {
            output_path: dir.out(),
            ..Default::default()
        },
    )
    .unwrap();

    // Prove it: a write on that connection is refused by the engine.
    let err = ex
        .conn_for_test()
        .execute("UPDATE qso SET call = 'HACKED'", [])
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("readonly") || msg.contains("read-only"),
        "expected a read-only refusal, got: {msg}"
    );

    // And the log is untouched.
    let s = LogStore::open(dir.db(), dir.adi()).unwrap();
    let n: i64 = s
        .conn()
        .query_row("SELECT COUNT(*) FROM qso WHERE call = 'HACKED'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn export_does_not_touch_updated_at() {
    let dir = TmpDir::new();
    let _s = store_with(&dir, fleet());

    let before: Vec<String> = {
        let s = LogStore::open(dir.db(), dir.adi()).unwrap();
        let mut stmt = s
            .conn()
            .prepare("SELECT updated_at FROM qso ORDER BY id")
            .unwrap();
        let v = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        v
    };

    let ex = Exporter::open(dir.db()).unwrap();
    ex.export(
        &ExportFilter::default(),
        &ExportOptions {
            output_path: dir.out(),
            ..Default::default()
        },
    )
    .unwrap();

    let after: Vec<String> = {
        let s = LogStore::open(dir.db(), dir.adi()).unwrap();
        let mut stmt = s
            .conn()
            .prepare("SELECT updated_at FROM qso ORDER BY id")
            .unwrap();
        let v = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        v
    };

    assert_eq!(before, after, "export must not restamp updated_at");
}

// ------------------------------------------------------------- streaming ---

#[test]
fn large_export_streams_without_materializing_the_log() {
    // 20k QSOs. The point isn't speed — it's that the writer holds one record at
    // a time, so peak memory is a buffer, not a heap of 20k records.
    //
    // Seeded with raw SQL in one transaction rather than through `insert()`:
    // `insert()` fsyncs the ADIF journal per contact (correct for a real QSO,
    // 20k fsyncs here), and this test is about the *export* path, not the insert
    // path — which the other tests cover.
    let dir = TmpDir::new();
    let mut s = LogStore::open(dir.db(), dir.adi()).unwrap();
    s.insert(&qso("SEED"), &station()).unwrap(); // creates the station row

    const N: usize = 20_000;
    {
        let conn = s.conn();
        conn.execute_batch("BEGIN").unwrap();
        let mut stmt = conn
            .prepare(
                "INSERT INTO qso (call,qso_date,time_on,band,mode,freq_hz,station_id,extra,\
                 created_at,updated_at) \
                 VALUES (?1,'20260711',?2,'20m','SSB',14207000,1,'{}','x','x')",
            )
            .unwrap();
        for i in 0..N - 1 {
            let time_on = format!("{:02}{:02}{:02}", i / 3600 % 24, (i / 60) % 60, i % 60);
            stmt.execute(rusqlite::params![format!("T{i:05}"), time_on])
                .unwrap();
        }
        drop(stmt);
        conn.execute_batch("COMMIT").unwrap();
    }
    drop(s);

    let ex = Exporter::open(dir.db()).unwrap();
    let summary = ex
        .export(
            &ExportFilter::default(),
            &ExportOptions {
                output_path: dir.out(),
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(summary.count, N);
    let text = std::fs::read_to_string(dir.out()).unwrap();
    assert_eq!(text.matches("<EOR>").count(), N);
    assert_eq!(summary.max_qso_id, Some(N as i64));
}

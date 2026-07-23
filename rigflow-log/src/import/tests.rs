//! Import tests. On-disk databases throughout — the plan runs on a read-only
//! connection in the client, and that is only real against a file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::dedupe::DEFAULT_WINDOW_SECS;
use crate::export::{ExportFilter, ExportOptions, Exporter};
use crate::model::Station;
use crate::store::LogStore;

// ---------------------------------------------------------------- fixtures --

static SEQ: AtomicU32 = AtomicU32::new(0);

struct TmpDir(PathBuf);

impl TmpDir {
    fn new() -> TmpDir {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("rigflow-log-import-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        TmpDir(dir)
    }
    fn db(&self) -> PathBuf {
        self.0.join("rigflow_log.db")
    }
    fn adi(&self) -> PathBuf {
        self.0.join("rigflow_log.adi")
    }
    fn out(&self) -> PathBuf {
        self.0.join("out.adi")
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
        ..Default::default()
    }
}

/// A minimal well-formed ADIF document with the given record bodies.
fn adif_doc(records: &[&str]) -> String {
    let mut s = String::from("<ADIF_VER:5>3.1.6 <EOH>\n");
    for r in records {
        s.push_str(r);
        s.push_str("<EOR>\n");
    }
    s
}

fn rec(call: &str, date: &str, time: &str, band: &str, mode: &str) -> String {
    format!(
        "<CALL:{}>{} <QSO_DATE:8>{} <TIME_ON:{}>{} <BAND:{}>{} <MODE:{}>{} ",
        call.len(),
        call,
        date,
        time.len(),
        time,
        band.len(),
        band,
        mode.len(),
        mode
    )
}

/// `rec` plus the QSL fields a LoTW confirmation report carries: `QSL_RCVD=Y`,
/// a `QSLRDATE`, and an `APP_LoTW_*` field that identifies the service.
fn lotw_rec(call: &str, date: &str, time: &str, band: &str, mode: &str, qslrdate: &str) -> String {
    format!(
        "{}<APP_LoTW_OWNCALL:6>KK7TCY <QSL_RCVD:1>Y <QSLRDATE:8>{} ",
        rec(call, date, time, band, mode),
        qslrdate,
    )
}

/// A W7WRO/40m/FT8 contact already in the log, matching `lotw_rec` above. Its
/// band is derived from frequency (canonical lowercase "40m").
fn insert_worked(store: &mut LogStore) {
    let mut q = crate::Qso {
        call: "W7WRO".into(),
        qso_date: "20260712".into(),
        time_on: "235600".into(),
        band: String::new(),
        mode: "FT8".into(),
        freq_hz: Some(7_076_000),
        ..Default::default()
    };
    q.normalize();
    store.insert(&q, &station()).unwrap();
}

fn plan_of(dir: &TmpDir, text: &str) -> ImportPlan {
    let store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let p = plan(store.conn(), text, DEFAULT_WINDOW_SECS).unwrap();
    p
}

// -------------------------------------------------------------------- plan --

#[test]
fn plan_reads_records_without_writing_anything() {
    let dir = TmpDir::new();
    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140000", "20m", "SSB"),
        &rec("K5ZD", "20260702", "150000", "40m", "CW"),
    ]);

    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.total, 2);
    assert_eq!(plan.importable.len(), 2);
    assert_eq!(plan.duplicates, 0);
    assert!(plan.unusable.is_empty());

    // Planning is read-only: the log is still empty.
    let store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let n: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM qso", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "plan must not write");
}

#[test]
fn plan_normalizes_on_the_way_in() {
    let dir = TmpDir::new();
    // USB is a sideband, not an ADIF mode; FT4 is MFSK/FT4. Both must be
    // canonicalized at import or they'd never match a filter or a dupe check.
    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140000", "20m", "USB"),
        &rec("EA1AAA", "20260701", "141000", "20m", "FT4"),
    ]);
    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.importable[0].mode, "SSB");
    assert_eq!(plan.importable[1].mode, "MFSK");
    assert_eq!(plan.importable[1].submode.as_deref(), Some("FT4"));
}

#[test]
fn plan_derives_band_from_freq_when_the_file_omits_it() {
    let dir = TmpDir::new();
    let doc = adif_doc(&[
        "<CALL:4>W1AW <QSO_DATE:8>20260701 <TIME_ON:6>140000 <MODE:3>SSB <FREQ:9>14.207000 ",
    ]);
    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.importable.len(), 1);
    assert_eq!(plan.importable[0].band, "20m");
    assert_eq!(plan.importable[0].freq_hz, Some(14_207_000));
}

#[test]
fn plan_skips_bad_records_and_names_them() {
    // A twenty-year log from another program will have cruft. Import the good
    // ones; do not refuse the file.
    let dir = TmpDir::new();
    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140000", "20m", "SSB"), // good
        "<QSO_DATE:8>20260701 <TIME_ON:6>141000 <BAND:3>20m <MODE:3>SSB ", // no CALL
        &rec("K5ZD", "notadate", "150000", "40m", "CW"),  // bad date
        &rec("VK3ABC", "20260702", "99", "15m", "SSB"),   // bad time
        "<CALL:5>JA1XY <QSO_DATE:8>20260703 <TIME_ON:6>160000 <BAND:3>10m ", // no MODE
        &rec("G0ABC", "20260704", "170000", "80m", "CW"), // good
    ]);

    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.total, 6);
    assert_eq!(plan.importable.len(), 2, "the two good ones");
    assert_eq!(plan.unusable.len(), 4);

    // Problems are identifiable: position in the file, and the call if it had one.
    assert_eq!(plan.unusable[0].record, 2);
    assert_eq!(plan.unusable[0].call, "");
    assert!(plan.unusable[0].reason.contains("CALL"));
    assert_eq!(plan.unusable[1].call, "K5ZD");
    assert!(plan.unusable[1].reason.contains("QSO_DATE"));
    assert!(plan.unusable[2].reason.contains("TIME_ON"));
    assert!(plan.unusable[3].reason.contains("MODE"));
}

#[test]
fn plan_skips_contacts_already_in_the_log() {
    let dir = TmpDir::new();
    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let mut existing = crate::Qso {
        call: "W1AW".into(),
        qso_date: "20260701".into(),
        time_on: "140000".into(),
        band: "20m".into(),
        mode: "SSB".into(),
        ..Default::default()
    };
    existing.normalize();
    store.insert(&existing, &station()).unwrap();
    drop(store);

    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140500", "20m", "SSB"), // within ±30min → dupe
        &rec("W1AW", "20260701", "190000", "20m", "SSB"), // hours later → NOT a dupe
        &rec("K5ZD", "20260702", "150000", "40m", "CW"),  // new
    ]);

    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.duplicates, 1);
    assert_eq!(plan.importable.len(), 2);
}

#[test]
fn plan_dedupes_an_uppercase_band_against_a_derived_lowercase_band() {
    // A contact stored with a frequency-derived band ("40m", lowercase) must
    // still dedupe against the same QSO re-imported from a program (LoTW,
    // WSJT-X, N1MM…) that writes BAND uppercase ("40M"). Regression: normalize()
    // once left a present band's case untouched, so the case-sensitive dedupe
    // key inserted the confirmation report's QSOs as fresh duplicates.
    let dir = TmpDir::new();
    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let mut existing = crate::Qso {
        call: "W7WRO".into(),
        qso_date: "20260712".into(),
        time_on: "235600".into(),
        band: String::new(), // derived from freq → canonical lowercase "40m"
        mode: "FT8".into(),
        freq_hz: Some(7_076_000),
        ..Default::default()
    };
    existing.normalize();
    assert_eq!(existing.band, "40m");
    store.insert(&existing, &station()).unwrap();
    drop(store);

    // Same QSO as it comes back in a LoTW report: BAND uppercase.
    let doc = adif_doc(&[&rec("W7WRO", "20260712", "235600", "40M", "FT8")]);
    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.duplicates, 1, "uppercase BAND must match stored 40m");
    assert_eq!(plan.importable.len(), 0);
}

#[test]
fn plan_treats_a_qsl_report_as_confirmations_not_new_contacts() {
    let dir = TmpDir::new();
    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    insert_worked(&mut store);
    drop(store);

    // The same QSO as it comes back from LoTW: uppercase BAND, QSL_RCVD=Y.
    let doc = adif_doc(&[&lotw_rec(
        "W7WRO", "20260712", "235600", "40M", "FT8", "20260723",
    )]);
    let plan = plan_of(&dir, &doc);
    assert_eq!(
        plan.importable.len(),
        0,
        "a confirmation is not a new contact"
    );
    assert_eq!(plan.duplicates, 0, "nor a plain duplicate");
    assert_eq!(plan.confirmations.len(), 1);
    let c = &plan.confirmations[0];
    assert_eq!(c.service, "lotw", "service inferred from APP_LoTW_* fields");
    assert_eq!(c.confirmed_at.as_deref(), Some("20260723"));
}

#[test]
fn committing_a_report_marks_the_contact_confirmed_and_is_idempotent() {
    let dir = TmpDir::new();
    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    insert_worked(&mut store);

    let doc = adif_doc(&[&lotw_rec(
        "W7WRO", "20260712", "235600", "40M", "FT8", "20260723",
    )]);
    let plan = plan(store.conn(), &doc, DEFAULT_WINDOW_SECS).unwrap();
    let outcome = store
        .commit_import(&plan.importable, &plan.confirmations, &station())
        .unwrap();
    assert_eq!(outcome.imported, 0, "no new rows");
    assert_eq!(outcome.confirmed, 1);

    // The contact view now shows the confirmation.
    let rows = store.query_contacts(10).unwrap();
    assert_eq!(rows.len(), 1, "still one contact, not two");
    assert_eq!(rows[0].confirmed, vec!["lotw".to_string()]);

    // Re-importing the same report records nothing new — idempotent preview.
    let plan2 = super::plan(store.conn(), &doc, DEFAULT_WINDOW_SECS).unwrap();
    assert_eq!(plan2.confirmations.len(), 0);
    assert_eq!(plan2.already_confirmed, 1);
    drop(store);

    // The client's contact view loads through the read-only Exporter, not the
    // store — the confirmation column must be populated there too.
    let ex = Exporter::open(dir.db()).unwrap();
    let page = ex
        .page(&ExportFilter::default(), 10, crate::export::Sort::Reverse)
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].confirmed, vec!["lotw".to_string()]);
}

#[test]
fn an_unmatched_confirmation_is_surfaced_not_inserted() {
    // A QSL for a QSO we never logged: counted, never turned into a phantom row.
    let dir = TmpDir::new();
    let doc = adif_doc(&[&lotw_rec(
        "W7WRO", "20260712", "235600", "40M", "FT8", "20260723",
    )]);
    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.unmatched_confirmations, 1);
    assert_eq!(plan.confirmations.len(), 0);
    assert_eq!(plan.importable.len(), 0);
}

#[test]
fn plan_catches_duplicates_within_the_file_itself() {
    // Another program's export can carry its own internal duplicates, and the DB
    // check cannot see rows we are about to add in the same batch.
    let dir = TmpDir::new();
    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140000", "20m", "SSB"),
        &rec("W1AW", "20260701", "140200", "20m", "SSB"), // same contact, again
        &rec("W1AW", "20260701", "140000", "40m", "SSB"), // different band → keep
    ]);
    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.total, 3);
    assert_eq!(plan.duplicates, 1);
    assert_eq!(plan.importable.len(), 2);
}

#[test]
fn a_file_that_is_not_adif_is_refused_outright() {
    // Per-record problems are survivable; a file we cannot tokenize is not.
    let dir = TmpDir::new();
    let store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let err = plan(store.conn(), "<CALL:99>W1AW <EOR>", DEFAULT_WINDOW_SECS);
    assert!(err.is_err(), "a truncated length must fail the whole parse");
}

#[test]
fn plan_of_an_empty_file_is_empty_not_an_error() {
    let dir = TmpDir::new();
    let plan = plan_of(&dir, "<ADIF_VER:5>3.1.6 <EOH>\n");
    assert_eq!(plan.total, 0);
    assert!(plan.is_empty());
}

#[test]
fn planning_works_on_a_read_only_connection() {
    // This is the path the CLIENT actually uses: the plan runs on the export
    // worker's read-only connection, off the UI thread. If it needed write access
    // it would fail only in production, never in the tests above (which plan on
    // the read-write store's connection).
    let dir = TmpDir::new();
    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let mut existing = crate::Qso {
        call: "W1AW".into(),
        qso_date: "20260701".into(),
        time_on: "140000".into(),
        band: "20m".into(),
        mode: "SSB".into(),
        ..Default::default()
    };
    existing.normalize();
    store.insert(&existing, &station()).unwrap();
    drop(store);

    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140500", "20m", "SSB"), // dupe of the above
        &rec("K5ZD", "20260702", "150000", "40m", "CW"),  // new
        &rec("", "20260703", "160000", "20m", "SSB"),     // unusable
    ]);

    let ex = Exporter::open(dir.db()).unwrap();
    let plan = ex.plan_import(&doc, DEFAULT_WINDOW_SECS).unwrap();

    assert_eq!(plan.total, 3);
    assert_eq!(plan.importable.len(), 1);
    assert_eq!(
        plan.duplicates, 1,
        "the dupe check queried the log read-only"
    );
    assert_eq!(plan.unusable.len(), 1);
}

// ------------------------------------------------------------------ commit --

#[test]
fn commit_writes_the_plan_and_the_journal() {
    let dir = TmpDir::new();
    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140000", "20m", "SSB"),
        &rec("K5ZD", "20260702", "150000", "40m", "CW"),
    ]);
    let plan = plan_of(&dir, &doc);

    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let outcome = store
        .commit_import(&plan.importable, &[], &station())
        .unwrap();
    assert_eq!(outcome.imported, 2);
    assert!(outcome.journal_appended);

    let rows = store.query_contacts(100).unwrap();
    assert_eq!(rows.len(), 2);

    // The journal carries both records, under one header.
    let journal = std::fs::read_to_string(dir.adi()).unwrap();
    assert_eq!(journal.matches("<EOH>").count(), 1);
    assert_eq!(journal.matches("<EOR>").count(), 2);
    assert!(journal.contains("W1AW") && journal.contains("K5ZD"));
}

#[test]
fn import_is_idempotent_reimporting_the_same_file_adds_nothing() {
    // The property that makes import safe to re-run. Without it, a nervous
    // operator importing twice would silently double their log.
    let dir = TmpDir::new();
    let doc = adif_doc(&[
        &rec("W1AW", "20260701", "140000", "20m", "SSB"),
        &rec("K5ZD", "20260702", "150000", "40m", "CW"),
    ]);

    let plan1 = plan_of(&dir, &doc);
    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    store
        .commit_import(&plan1.importable, &[], &station())
        .unwrap();
    drop(store);

    let plan2 = plan_of(&dir, &doc);
    assert_eq!(plan2.total, 2);
    assert_eq!(plan2.duplicates, 2, "both already logged");
    assert!(plan2.importable.is_empty());

    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let outcome = store
        .commit_import(&plan2.importable, &[], &station())
        .unwrap();
    assert_eq!(outcome.imported, 0);
    assert_eq!(store.query_contacts(100).unwrap().len(), 2, "still 2");
}

#[test]
fn an_imported_records_own_my_fields_survive_the_station_snapshot() {
    // Importing another log's QSOs must not rewrite where THEY were made from.
    // The station fills only what the record doesn't already carry.
    let dir = TmpDir::new();
    let doc = adif_doc(&[
        &format!(
            "{}<MY_GRIDSQUARE:4>FN31 <STATION_CALLSIGN:5>W9XYZ ",
            rec("W1AW", "20260701", "140000", "20m", "SSB")
        ),
        // …and a record with no MY_* at all picks up the current station.
        &rec("K5ZD", "20260702", "150000", "40m", "CW"),
    ]);
    let plan = plan_of(&dir, &doc);

    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    store
        .commit_import(&plan.importable, &[], &station())
        .unwrap();

    let rows = store.query_contacts(100).unwrap();
    let by_call = |c: &str| {
        rows.iter()
            .find(|r| r.qso.call == c)
            .unwrap()
            .qso
            .extra
            .clone()
    };

    let imported = by_call("W1AW");
    assert_eq!(
        imported.get("MY_GRIDSQUARE").map(String::as_str),
        Some("FN31"),
        "the file's own grid is historical truth and must not be overwritten"
    );
    assert_eq!(
        imported.get("STATION_CALLSIGN").map(String::as_str),
        Some("W9XYZ")
    );

    let bare = by_call("K5ZD");
    assert_eq!(
        bare.get("MY_GRIDSQUARE").map(String::as_str),
        Some("EM12"),
        "a record with no MY_* gets the current station"
    );
}

#[test]
fn commit_is_atomic_a_failing_row_imports_nothing() {
    // Half an import is worse than none: the operator cannot tell which half
    // landed. Prove the ROLLBACK, not the validation — so force a failure from
    // inside SQLite, on a row the planner would have happily accepted.
    let dir = TmpDir::new();
    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    store
        .conn()
        .execute_batch(
            "CREATE TRIGGER boom BEFORE INSERT ON qso WHEN NEW.call = 'BOOM' \
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();

    let q = |call: &str, time: &str| {
        let mut q = crate::Qso {
            call: call.into(),
            qso_date: "20260701".into(),
            time_on: time.into(),
            band: "20m".into(),
            mode: "SSB".into(),
            ..Default::default()
        };
        q.normalize();
        q
    };

    // The bad row sits in the MIDDLE: rows before it have already been inserted
    // inside the transaction when it blows up.
    let batch = [
        q("W1AW", "140000"),
        q("BOOM", "150000"),
        q("K5ZD", "160000"),
    ];

    let result = store.commit_import(&batch, &[], &station());
    assert!(result.is_err(), "the aborting row must fail the commit");

    let rows = store.query_contacts(100).unwrap();
    assert!(
        rows.is_empty(),
        "the whole batch must roll back — found {} row(s), so a partial import \
         was committed",
        rows.len()
    );
}

// -------------------------------------------------------------- round-trip --

#[test]
fn export_then_import_into_a_clean_log_round_trips() {
    // The two halves must agree: what export writes, import must read back
    // identically — including split fields, submodes, and extras.
    let src = TmpDir::new();
    let mut store = LogStore::open(src.db(), src.adi()).unwrap();

    let mut split = crate::Qso {
        call: "DX1SPLIT".into(),
        qso_date: "20260701".into(),
        time_on: "140000".into(),
        band: "20m".into(),
        mode: "CW".into(),
        freq_hz: Some(14_025_000),
        freq_rx_hz: Some(14_030_000),
        rst_sent: Some("599".into()),
        ..Default::default()
    };
    split.normalize();

    let mut ft4 = crate::Qso {
        call: "EA1AAA".into(),
        qso_date: "20260702".into(),
        time_on: "150000".into(),
        mode: "FT4".into(),
        freq_hz: Some(14_080_000),
        ..Default::default()
    };
    ft4.normalize();

    let mut intl = crate::Qso {
        call: "OH2ÄÄ".into(),
        qso_date: "20260703".into(),
        time_on: "160000".into(),
        band: "40m".into(),
        mode: "SSB".into(),
        ..Default::default()
    };
    intl.extra.insert("NAME_INTL".into(), "Jörg".into());
    intl.normalize();

    for q in [&split, &ft4, &intl] {
        store.insert(q, &station()).unwrap();
    }

    let ex = Exporter::open(src.db()).unwrap();
    ex.export(
        &ExportFilter::default(),
        &ExportOptions {
            output_path: src.out(),
            ..Default::default()
        },
    )
    .unwrap();
    let text = std::fs::read_to_string(src.out()).unwrap();

    // …into a clean log, through the real import path.
    let dst = TmpDir::new();
    let plan = plan_of(&dst, &text);
    assert_eq!(plan.total, 3);
    assert_eq!(plan.importable.len(), 3);
    assert!(
        plan.unusable.is_empty(),
        "our own export must import cleanly"
    );

    let mut dst_store = LogStore::open(dst.db(), dst.adi()).unwrap();
    dst_store
        .commit_import(&plan.importable, &[], &station())
        .unwrap();

    let rows = dst_store.query_contacts(100).unwrap();
    assert_eq!(rows.len(), 3);
    let by_call = |c: &str| rows.iter().find(|r| r.qso.call == c).unwrap().qso.clone();

    let back = by_call("DX1SPLIT");
    assert_eq!(back.freq_rx_hz, Some(14_030_000));
    assert_eq!(back.band_rx.as_deref(), Some("20m"));
    assert_eq!(back.rst_sent.as_deref(), Some("599"));

    let back = by_call("EA1AAA");
    assert_eq!(back.mode, "MFSK");
    assert_eq!(
        back.submode.as_deref(),
        Some("FT4"),
        "FT4 survives the trip"
    );

    let back = by_call("OH2ÄÄ");
    assert_eq!(
        back.extra.get("NAME_INTL").map(String::as_str),
        Some("Jörg")
    );
}

// --------------------------------------------------------------- bulk path --

#[test]
fn a_large_import_is_one_transaction_not_a_loop_over_insert() {
    // 20k records. `insert()` fsyncs per contact (right for a live QSO, ~100s for
    // 20k); commit_import pays that once. If this test ever crawls, someone has
    // turned the bulk path back into a loop over insert().
    let dir = TmpDir::new();
    let mut records = Vec::new();
    for i in 0..20_000u32 {
        records.push(rec(
            &format!("T{i:05}"),
            "20260701",
            &format!("{:02}{:02}{:02}", i / 3600 % 24, (i / 60) % 60, i % 60),
            "20m",
            "SSB",
        ));
    }
    let doc = adif_doc(&records.iter().map(String::as_str).collect::<Vec<_>>());

    let started = std::time::Instant::now();
    let plan = plan_of(&dir, &doc);
    assert_eq!(plan.importable.len(), 20_000);

    let mut store = LogStore::open(dir.db(), dir.adi()).unwrap();
    let outcome = store
        .commit_import(&plan.importable, &[], &station())
        .unwrap();
    assert_eq!(outcome.imported, 20_000);

    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "20k import took {elapsed:?} — the bulk path has regressed to per-row fsync"
    );

    // One header, 20k records, one journal write.
    let journal = std::fs::read_to_string(dir.adi()).unwrap();
    assert_eq!(journal.matches("<EOR>").count(), 20_000);
    assert_eq!(journal.matches("<EOH>").count(), 1);
}

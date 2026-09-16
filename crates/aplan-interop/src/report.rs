//! What an import did, and what it could not carry.
//!
//! ADR-0012 clause 3: every field the task model does not hold produces a
//! notice. Silent omission is forbidden, because a lossy import that quietly
//! drops a field is precisely the drift this tool exists to detect (HLD
//! section 13).
//!
//! These notices are deliberately *not* [`aplan_core::Diagnostic`]s: they
//! describe a conversion, not a violated invariant in a plan, so they carry no
//! `APLAN0xx` code and never enter the validator's vocabulary.

use std::fmt;

/// Why a notice was raised. The kind orders the report, so a reader sees what
/// was lost before what was merely arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NoticeKind {
    /// A source field the task model does not hold, so it did not survive.
    DroppedField,
    /// How strongly the imported claim is backed, and what would prove it.
    Provenance,
    /// Where the imported tasks landed, and what the reader must still do.
    Placement,
}

impl NoticeKind {
    /// Every kind, so a test can walk the set rather than trust a hand-written
    /// list (the coverage-drift discipline `Rule::ALL` and `Format::ALL` use).
    pub const ALL: &'static [NoticeKind] = &[
        NoticeKind::DroppedField,
        NoticeKind::Provenance,
        NoticeKind::Placement,
    ];

    /// Stable label used when rendering the report.
    pub const fn label(self) -> &'static str {
        match self {
            NoticeKind::DroppedField => "dropped",
            NoticeKind::Provenance => "provenance",
            NoticeKind::Placement => "placement",
        }
    }
}

/// One thing the reader needs to know about an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The document this arose from, as the user named it: the source for an
    /// import, the destination for an export.
    pub source: String,
    /// What sort of notice this is.
    pub kind: NoticeKind,
    /// Human-readable detail, one line.
    pub message: String,
}

impl Notice {
    /// Build a notice for `source`.
    pub fn new(source: impl Into<String>, kind: NoticeKind, message: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            kind,
            message: message.into(),
        }
    }
}

/// Everything an import wants to tell the user, in one place.
///
/// Deterministic by construction: notices are held in the order they were
/// recorded, and rendering never consults a hash map, so the same input yields
/// byte-identical output (HLD section 14).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    notices: Vec<Notice>,
}

impl Report {
    /// An empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a notice.
    pub fn push(&mut self, notice: Notice) {
        self.notices.push(notice);
    }

    /// Record that `field` did not survive the conversion.
    pub fn dropped(&mut self, source: impl Into<String>, field: &str) {
        self.push(Notice::new(
            source,
            NoticeKind::DroppedField,
            format!("`{field}` is not held by the task model"),
        ));
    }

    /// Record that `field` has no place in the shape being *written*.
    ///
    /// The mirror of [`Report::dropped`], and deliberately a second method
    /// rather than a reworded first: an import loses what the task model cannot
    /// hold, an export loses what the target shape cannot, and a reader of the
    /// report has to be able to tell which end the loss happened at.
    pub fn not_carried(&mut self, destination: impl Into<String>, field: &str, detail: &str) {
        self.push(Notice::new(
            destination,
            NoticeKind::DroppedField,
            format!("`{field}` has no place in this shape: {detail}"),
        ));
    }

    /// Every notice, in the order recorded.
    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    /// Whether anything was recorded at all.
    pub fn is_empty(&self) -> bool {
        self.notices.is_empty()
    }

    /// How many notices of `kind` were recorded.
    pub fn count(&self, kind: NoticeKind) -> usize {
        self.notices.iter().filter(|n| n.kind == kind).count()
    }

    /// Fold another report's notices into this one, preserving order.
    pub fn extend(&mut self, other: Report) {
        self.notices.extend(other.notices);
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for n in &self.notices {
            writeln!(f, "  {}: {} ({})", n.kind.label(), n.message, n.source)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_field_names_the_field_and_its_source() {
        let mut r = Report::new();
        r.dropped("kep.yaml", "approvers");
        assert_eq!(r.count(NoticeKind::DroppedField), 1);
        let n = &r.notices()[0];
        assert_eq!(n.source, "kep.yaml");
        assert!(n.message.contains("approvers"), "{}", n.message);
    }

    #[test]
    fn a_field_the_target_shape_cannot_hold_names_the_field_and_the_destination() {
        // The export half of the same honesty rule, and it has to be
        // distinguishable from the import half in the rendered report: one says
        // the model could not hold it, the other that the shape could not.
        let mut r = Report::new();
        r.not_carried("ROADMAP.md", "evidence", "3 task(s) carry it");
        let n = &r.notices()[0];
        assert_eq!(n.kind, NoticeKind::DroppedField);
        assert_eq!(n.source, "ROADMAP.md");
        assert!(n.message.contains("evidence"), "{}", n.message);
        assert!(n.message.contains("this shape"), "{}", n.message);

        let mut other = Report::new();
        other.dropped("ROADMAP.md", "evidence");
        assert_ne!(
            other.notices()[0].message,
            n.message,
            "an import loss and an export loss must not read identically"
        );
    }

    #[test]
    fn notices_keep_their_recorded_order() {
        let mut r = Report::new();
        r.dropped("a.yaml", "first");
        r.push(Notice::new("a.yaml", NoticeKind::Provenance, "second"));
        r.dropped("a.yaml", "third");
        let messages: Vec<&str> = r.notices().iter().map(|n| n.message.as_str()).collect();
        assert!(messages[0].contains("first"));
        assert_eq!(messages[1], "second");
        assert!(messages[2].contains("third"));
    }

    #[test]
    fn every_kind_has_a_distinct_non_empty_label() {
        // Walks the set rather than a hand-written list: a kind added without a
        // usable label fails here, the way `Rule::ALL` guards the rule codes.
        let mut seen = std::collections::BTreeSet::new();
        for kind in NoticeKind::ALL {
            let label = kind.label();
            assert!(!label.is_empty(), "{kind:?} has an empty label");
            assert!(seen.insert(label), "{kind:?} reuses the label {label:?}");
        }
        assert_eq!(seen.len(), NoticeKind::ALL.len());
    }

    #[test]
    fn an_empty_report_renders_to_nothing() {
        assert!(Report::new().is_empty());
        assert_eq!(Report::new().to_string(), "");
    }

    #[test]
    fn extend_preserves_order_across_reports() {
        let mut a = Report::new();
        a.dropped("a.yaml", "one");
        let mut b = Report::new();
        b.dropped("b.yaml", "two");
        a.extend(b);
        assert_eq!(a.notices().len(), 2);
        assert_eq!(a.notices()[1].source, "b.yaml");
    }
}

//! Résultats d'une exécution de la suite et leur rendu lisible.

use std::fmt::Write as _;

/// Verdict d'une vérification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Le comportement observé respecte la spécification.
    Pass,
    /// Écart à la spécification, avec l'explication.
    Fail(String),
    /// Vérification non applicable ou impossible à observer en boîte noire.
    Skip(String),
}

/// Une vérification exécutée et son verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckResult {
    /// Identifiant stable (`HOST-02`), cité dans la spec et les rapports.
    pub id: &'static str,
    /// Section de `docs/wire-protocol.md` qui porte l'exigence.
    pub section: &'static str,
    pub title: &'static str,
    pub outcome: Outcome,
}

/// Rapport complet d'une exécution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub results: Vec<CheckResult>,
}

impl Report {
    pub(crate) fn record(
        &mut self,
        id: &'static str,
        section: &'static str,
        title: &'static str,
        outcome: Outcome,
    ) {
        self.results.push(CheckResult {
            id,
            section,
            title,
            outcome,
        });
    }

    fn count(&self, pick: impl Fn(&Outcome) -> bool) -> usize {
        self.results.iter().filter(|r| pick(&r.outcome)).count()
    }

    pub fn passed(&self) -> usize {
        self.count(|o| matches!(o, Outcome::Pass))
    }

    pub fn failed(&self) -> usize {
        self.count(|o| matches!(o, Outcome::Fail(_)))
    }

    pub fn skipped(&self) -> usize {
        self.count(|o| matches!(o, Outcome::Skip(_)))
    }

    /// Vrai quand aucune vérification n'a échoué (les `Skip` ne comptent pas).
    pub fn is_conformant(&self) -> bool {
        self.failed() == 0
    }

    /// Vrai quand aucune vérification n'a échoué **ni été ignorée** : toute la
    /// spécification a été observée.
    pub fn is_fully_checked(&self) -> bool {
        self.failed() == 0 && self.skipped() == 0
    }

    /// Rapport texte, une ligne par vérification puis un bilan.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for r in &self.results {
            let (tag, detail) = match &r.outcome {
                Outcome::Pass => ("OK  ", None),
                Outcome::Fail(why) => ("FAIL", Some(why)),
                Outcome::Skip(why) => ("SKIP", Some(why)),
            };
            let _ = writeln!(out, "[{tag}] {:<9} {:<5} {}", r.id, r.section, r.title);
            if let Some(why) = detail {
                let _ = writeln!(out, "       -> {why}");
            }
        }
        let _ = writeln!(
            out,
            "\n{} vérifications : {} réussies, {} en échec, {} ignorées",
            self.results.len(),
            self.passed(),
            self.failed(),
            self.skipped()
        );
        let _ = writeln!(
            out,
            "{}",
            if self.is_conformant() {
                "Verdict : CONFORME au protocole."
            } else {
                "Verdict : NON CONFORME."
            }
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_et_comptes() {
        let mut report = Report::default();
        report.record("A-01", "§1", "ok", Outcome::Pass);
        report.record("A-02", "§1", "ignoré", Outcome::Skip("raison".into()));
        assert!(report.is_conformant());
        assert!(!report.is_fully_checked());
        report.record("A-03", "§1", "ko", Outcome::Fail("écart".into()));
        assert!(!report.is_conformant());
        assert_eq!(
            (report.passed(), report.failed(), report.skipped()),
            (1, 1, 1)
        );
        let text = report.render();
        assert!(text.contains("[FAIL] A-03"));
        assert!(text.contains("NON CONFORME"));
    }
}

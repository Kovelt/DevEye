//! Tests de cohérence entre la documentation et le code (ADR-0048).
//!
//! Ils lisent les fichiers Markdown du dépôt et échouent quand la doc décrit
//! autre chose que ce que le code fait : liste des outils du README et de
//! `docs/usage.md`, nombres d'outils annoncés, liens relatifs et ancres, index
//! des ADR. Rien n'est généré : une doc qui dérive casse la CI et se corrige à
//! la main, au même endroit que le code. Les archives de `docs/historique/`
//! sont exclues (documents datés, non normatifs).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::*;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("lecture de {path:?} : {e}"))
}

fn registered_tools() -> BTreeSet<String> {
    DeveyeMcpServer::tool_router()
        .list_all()
        .iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// Outils stables : les enregistrés moins les expérimentaux (ADR-0069), seuls
/// promis par la liste du README et par les nombres annoncés.
fn stable_tools() -> BTreeSet<String> {
    registered_tools()
        .into_iter()
        .filter(|name| !EXPERIMENTAL_TOOLS.contains(&name.as_str()))
        .collect()
}

/// Segments entre backticks d'une ligne (les segments d'indice impair).
fn backticked(line: &str) -> Vec<&str> {
    line.split('`').skip(1).step_by(2).collect()
}

#[test]
fn readme_lists_exactly_the_registered_tools() {
    let readme = read("README.md");
    let section = readme
        .split_once("<!-- outils:debut -->")
        .and_then(|(_, rest)| rest.split_once("<!-- outils:fin -->"))
        .map(|(section, _)| section)
        .expect("le README doit encadrer la liste des outils par <!-- outils:debut --> et <!-- outils:fin -->");
    let mut documented = BTreeSet::new();
    for line in section.lines().filter(|l| l.starts_with("| **")) {
        for name in backticked(line) {
            assert!(
                documented.insert(name.to_string()),
                "outil `{name}` cité deux fois dans le README"
            );
        }
    }
    let registered = stable_tools();
    let missing: Vec<_> = registered.difference(&documented).collect();
    let unknown: Vec<_> = documented.difference(&registered).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "README et outils stables divergent : absents du README {missing:?}, inconnus du code {unknown:?}"
    );
}

#[test]
fn usage_documents_each_registered_tool() {
    let usage = read("docs/usage.md");
    let mut documented = BTreeSet::new();
    for line in usage.lines().filter(|l| l.starts_with("#### `")) {
        let name = backticked(line)
            .first()
            .copied()
            .unwrap_or_else(|| panic!("titre d'outil sans nom : {line}"));
        assert!(
            documented.insert(name.to_string()),
            "outil `{name}` documenté deux fois dans docs/usage.md"
        );
    }
    let registered = registered_tools();
    let missing: Vec<_> = registered.difference(&documented).collect();
    let unknown: Vec<_> = documented.difference(&registered).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "docs/usage.md et outils enregistrés divergent : absents {missing:?}, inconnus du code {unknown:?}"
    );
}

#[test]
fn announced_tool_counts_match_the_code() {
    assert_eq!(registered_tools().len(), EXPECTED_TOOL_COUNT);
    let registered = stable_tools().len();
    assert_eq!(registered, STABLE_TOOL_COUNT);
    for file in ["README.md", "docs/usage.md", "docs/migration-1.0.md"] {
        let text = read(file);
        let mut seen = 0;
        for (index, _) in text.match_indices(" outils") {
            let before = &text[..index];
            let word = before
                .rsplit(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or_default();
            if word.is_empty() {
                continue;
            }
            seen += 1;
            assert_eq!(
                word,
                registered.to_string(),
                "{file} annonce « {word} outils » : le code en enregistre {registered} stables"
            );
        }
        assert!(
            seen > 0 || file == "docs/migration-1.0.md",
            "{file} n'annonce aucun nombre d'outils"
        );
    }
}

/// Les outils expérimentaux portent leur statut dans leur description, sont
/// documentés dans `docs/usage.md` comme hors promesse et listés par
/// `docs/stability.md` (ADR-0069).
#[test]
fn experimental_tools_are_flagged_and_documented_as_such() {
    let tools = DeveyeMcpServer::tool_router().list_all();
    let stability = read("docs/stability.md");
    let usage = read("docs/usage.md");
    for name in EXPERIMENTAL_TOOLS {
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == *name)
            .unwrap_or_else(|| panic!("l'outil expérimental `{name}` n'est pas enregistré"));
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(
            description.starts_with(EXPERIMENTAL_PREFIX),
            "la description de `{name}` doit commencer par {EXPERIMENTAL_PREFIX}"
        );
        assert!(
            stability.contains(&format!("`{name}`")),
            "docs/stability.md doit lister l'outil expérimental `{name}`"
        );
        assert!(
            usage.contains(&format!("#### `{name}`")),
            "docs/usage.md doit documenter `{name}`"
        );
    }
    for tool in &tools {
        let description = tool.description.as_deref().unwrap_or_default();
        assert_eq!(
            description.starts_with(EXPERIMENTAL_PREFIX),
            EXPERIMENTAL_TOOLS.contains(&tool.name.as_ref()),
            "`{}` : le préfixe {EXPERIMENTAL_PREFIX} et EXPERIMENTAL_TOOLS doivent concorder",
            tool.name
        );
    }
}

/// Fichiers Markdown du dépôt, hors archives datées, dépendances et build.
fn markdown_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !matches!(
                    name.as_str(),
                    "target" | ".git" | "historique" | "node_modules"
                ) {
                    walk(&path, out);
                }
            } else if name.ends_with(".md") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo_root(), &mut files);
    files.sort();
    files
}

/// Ancre GitHub d'un titre : minuscules, lettres, chiffres, `-` et `_`
/// conservés, espaces en tirets, tout le reste supprimé.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn anchors_of(text: &str) -> BTreeSet<String> {
    let mut in_fence = false;
    let mut anchors = BTreeSet::new();
    for line in text.lines() {
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        if in_fence {
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            anchors.insert(slug(&line[hashes..].replace(['`', '*'], "")));
        }
    }
    anchors
}

/// Cibles `](cible)` d'un fichier, hors blocs de code et hors extraits `code`.
fn link_targets(text: &str) -> Vec<(usize, String)> {
    let mut in_fence = false;
    let mut targets = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        if in_fence {
            continue;
        }
        let visible: String = line.split('`').step_by(2).collect::<Vec<_>>().join(" ");
        let mut rest = visible.as_str();
        while let Some(start) = rest.find("](") {
            let after = &rest[start + 2..];
            let Some(end) = after.find(')') else { break };
            let target = after[..end].trim();
            if !target.is_empty() {
                targets.push((number + 1, target.to_string()));
            }
            rest = &after[end..];
        }
    }
    targets
}

#[test]
fn relative_links_and_anchors_resolve() {
    let root = repo_root();
    let mut broken = Vec::new();
    for file in markdown_files() {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for (line, target) in link_targets(&text) {
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            let (path, fragment) = match target.split_once('#') {
                Some((p, f)) => (p, Some(f)),
                None => (target.as_str(), None),
            };
            let resolved = if path.is_empty() {
                file.clone()
            } else {
                file.parent().unwrap_or(&root).join(path)
            };
            let shown = file.strip_prefix(&root).unwrap_or(&file).display();
            if !resolved.exists() {
                broken.push(format!("{shown}:{line} : fichier introuvable : {target}"));
                continue;
            }
            if let Some(fragment) = fragment {
                if resolved.extension().is_some_and(|e| e == "md") {
                    let linked = std::fs::read_to_string(&resolved).unwrap_or_default();
                    if !anchors_of(&linked).contains(fragment) {
                        broken.push(format!("{shown}:{line} : ancre introuvable : {target}"));
                    }
                }
            }
        }
    }
    assert!(broken.is_empty(), "liens cassés :\n{}", broken.join("\n"));
}

/// Valeur d'une puce `- **Clé** : valeur` d'un ADR.
fn adr_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("- **{key}** : ");
    text.lines().find_map(|l| l.strip_prefix(prefix.as_str()))
}

#[test]
fn every_adr_is_indexed_with_its_title_and_status() {
    let dir = repo_root().join("docs/adr");
    let index = read("docs/adr/README.md");
    let mut numbers = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("docs/adr lisible").flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(number) = name
            .get(..4)
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        if !name.ends_with(".md") {
            continue;
        }
        numbers.push(number.parse::<u32>().expect("numéro d'ADR"));
        let text = std::fs::read_to_string(entry.path()).expect("ADR lisible");
        let title = text
            .lines()
            .next()
            .and_then(|l| l.strip_prefix(&format!("# ADR-{number} — ")))
            .unwrap_or_else(|| panic!("{name} : le titre doit être « # ADR-{number} — … »"));
        let status = adr_field(&text, "Statut").unwrap_or_else(|| panic!("{name} : sans statut"));
        let row = format!("| [{number}]({name}) | {title} | {status} |");
        assert!(
            index.lines().any(|l| l == row),
            "docs/adr/README.md : ligne d'index absente ou différente pour {name}, attendue : {row}"
        );
    }
    numbers.sort_unstable();
    let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
    assert_eq!(
        numbers, expected,
        "la numérotation des ADR doit être continue, sans trou"
    );
}

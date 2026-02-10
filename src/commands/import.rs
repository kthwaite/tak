use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use colored::Colorize;
use serde::{Deserialize, Serialize};

use crate::commands::create::derive_traceability;
use crate::error::{Result, TakError};
use crate::model::{Contract, Estimate, Kind, Planning, Priority, Risk, Task};
use crate::output::{self, Format};
use crate::store::repo::{Repo, resolve_task_id_input};
use crate::task_id::TaskId;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ImportDocument {
    Wrapped { tasks: Vec<ImportTaskSpec> },
    List(Vec<ImportTaskSpec>),
}

impl ImportDocument {
    fn into_tasks(self) -> Vec<ImportTaskSpec> {
        match self {
            Self::Wrapped { tasks } => tasks,
            Self::List(tasks) => tasks,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ImportTaskSpec {
    #[serde(default, alias = "key", alias = "ref")]
    id: Option<String>,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    kind: Kind,
    #[serde(default, alias = "tag")]
    tags: Vec<String>,
    #[serde(default)]
    parent: Option<ImportRef>,
    #[serde(default, alias = "depends", alias = "depends-on")]
    depends_on: Vec<ImportRef>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(
        default,
        alias = "criterion",
        alias = "criteria",
        alias = "acceptance_criteria"
    )]
    acceptance_criteria: Vec<String>,
    #[serde(default, alias = "verify", alias = "verification")]
    verification: Vec<String>,
    #[serde(default, alias = "constraint", alias = "constraints")]
    constraints: Vec<String>,
    #[serde(default)]
    priority: Option<Priority>,
    #[serde(default)]
    estimate: Option<Estimate>,
    #[serde(default)]
    risk: Option<Risk>,
    #[serde(default, alias = "skill", alias = "required_skills")]
    required_skills: Vec<String>,
    #[serde(default)]
    children: Vec<ImportTaskSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ImportRef {
    String(String),
    Number(u64),
}

impl ImportRef {
    fn as_text(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Number(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct FlatTaskSpec {
    index: usize,
    alias: Option<String>,
    title: String,
    description: Option<String>,
    kind: Kind,
    tags: Vec<String>,
    explicit_parent: Option<String>,
    nested_parent: Option<usize>,
    depends_on: Vec<String>,
    contract: Contract,
    planning: Planning,
}

#[derive(Debug, Clone, Copy)]
enum ResolvedRef {
    Local(usize),
    Existing(u64),
}

#[derive(Debug, Clone)]
struct ResolvedTaskSpec {
    flat: FlatTaskSpec,
    parent: Option<ResolvedRef>,
    depends_on: Vec<ResolvedRef>,
}

#[derive(Debug, Serialize)]
struct DryRunTaskPreview {
    order: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    title: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DryRunReport {
    dry_run: bool,
    source: String,
    task_count: usize,
    tasks: Vec<DryRunTaskPreview>,
}

pub fn run(repo_root: &Path, source: String, dry_run: bool, format: Format) -> Result<()> {
    let repo = Repo::open(repo_root)?;
    let raw = read_source(&source)?;
    let parsed = parse_document(&source, &raw)?;
    let flat = flatten_tasks(parsed)?;

    let existing_ids: Vec<TaskId> = repo
        .store
        .list_ids()?
        .into_iter()
        .map(TaskId::from)
        .collect();
    let resolved = resolve_tasks(flat, &existing_ids)?;
    let order = build_creation_order(&resolved)?;

    if dry_run {
        print_dry_run_report(&source, &resolved, &order, format)?;
        return Ok(());
    }

    let created = apply_import(&repo, &resolved, &order)?;
    output::print_tasks(&created, format)?;
    Ok(())
}

fn invalid_spec(message: impl Into<String>) -> TakError {
    TakError::ImportInvalidSpec(message.into())
}

fn read_source(source: &str) -> Result<String> {
    if source == "-" {
        let mut stdin = std::io::stdin();
        let mut contents = String::new();
        stdin.read_to_string(&mut contents)?;
        return Ok(contents);
    }

    Ok(fs::read_to_string(source)?)
}

fn parse_document(source: &str, raw: &str) -> Result<Vec<ImportTaskSpec>> {
    if raw.trim().is_empty() {
        return Err(invalid_spec(format!("source '{source}' is empty")));
    }

    let doc = match serde_json::from_str::<ImportDocument>(raw) {
        Ok(doc) => doc,
        Err(json_err) => match serde_yaml::from_str::<ImportDocument>(raw) {
            Ok(doc) => doc,
            Err(yaml_err) => {
                return Err(invalid_spec(format!(
                    "failed to parse '{source}' as JSON ({json_err}) or YAML ({yaml_err})"
                )));
            }
        },
    };

    let tasks = doc.into_tasks();
    if tasks.is_empty() {
        return Err(invalid_spec(format!(
            "source '{source}' has no tasks to import"
        )));
    }

    Ok(tasks)
}

fn normalize_reference(reference: &ImportRef) -> Result<String> {
    let raw = reference.as_text();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid_spec("reference values cannot be empty"));
    }
    Ok(trimmed.to_string())
}

fn flatten_tasks(tasks: Vec<ImportTaskSpec>) -> Result<Vec<FlatTaskSpec>> {
    let mut flat = Vec::new();
    for task in tasks {
        flatten_task(task, None, &mut flat)?;
    }

    if flat.is_empty() {
        return Err(invalid_spec("import payload did not yield any tasks"));
    }

    Ok(flat)
}

fn flatten_task(
    task: ImportTaskSpec,
    nested_parent: Option<usize>,
    out: &mut Vec<FlatTaskSpec>,
) -> Result<()> {
    let title = task.title.trim().to_string();
    if title.is_empty() {
        return Err(invalid_spec("task title cannot be empty"));
    }

    if nested_parent.is_some() && task.parent.is_some() {
        return Err(invalid_spec(format!(
            "task '{title}' is nested under a parent but also sets explicit parent"
        )));
    }

    let alias = task.id.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let explicit_parent = task.parent.as_ref().map(normalize_reference).transpose()?;
    let depends_on = task
        .depends_on
        .iter()
        .map(normalize_reference)
        .collect::<Result<Vec<_>>>()?;

    let index = out.len();
    out.push(FlatTaskSpec {
        index,
        alias,
        title,
        description: task.description,
        kind: task.kind,
        tags: task.tags,
        explicit_parent,
        nested_parent,
        depends_on,
        contract: Contract {
            objective: task.objective,
            acceptance_criteria: task.acceptance_criteria,
            verification: task.verification,
            constraints: task.constraints,
        },
        planning: Planning {
            priority: task.priority,
            estimate: task.estimate,
            required_skills: task.required_skills,
            risk: task.risk,
        },
    });

    for child in task.children {
        flatten_task(child, Some(index), out)?;
    }

    Ok(())
}

fn build_alias_map(tasks: &[FlatTaskSpec]) -> Result<HashMap<String, usize>> {
    let mut aliases = HashMap::new();

    for task in tasks {
        if let Some(alias) = task.alias.as_deref() {
            let trimmed = alias.trim();
            if trimmed.is_empty() {
                return Err(invalid_spec(format!(
                    "task '{}' has an empty alias",
                    task.title
                )));
            }
            if let Some(existing_index) = aliases.insert(trimmed.to_string(), task.index) {
                let existing_label = task_label(tasks, existing_index);
                return Err(invalid_spec(format!(
                    "duplicate alias '{trimmed}' used by '{}' and '{existing_label}'",
                    task.title
                )));
            }
        }
    }

    Ok(aliases)
}

fn resolve_reference(
    raw: &str,
    aliases: &HashMap<String, usize>,
    existing_ids: &[TaskId],
) -> Result<ResolvedRef> {
    if let Some(index) = aliases.get(raw).copied() {
        return Ok(ResolvedRef::Local(index));
    }

    let resolved = resolve_task_id_input(raw, existing_ids).map_err(|err| {
        invalid_spec(format!(
            "reference '{raw}' does not match an imported alias or existing task id: {err}"
        ))
    })?;

    Ok(ResolvedRef::Existing(resolved.into()))
}

fn resolve_tasks(
    flat: Vec<FlatTaskSpec>,
    existing_ids: &[TaskId],
) -> Result<Vec<ResolvedTaskSpec>> {
    let aliases = build_alias_map(&flat)?;
    let mut resolved = Vec::with_capacity(flat.len());

    for task in flat {
        let parent = if let Some(parent_index) = task.nested_parent {
            Some(ResolvedRef::Local(parent_index))
        } else if let Some(raw_parent) = task.explicit_parent.as_deref() {
            Some(resolve_reference(raw_parent, &aliases, existing_ids)?)
        } else {
            None
        };

        if matches!(parent, Some(ResolvedRef::Local(idx)) if idx == task.index) {
            return Err(invalid_spec(format!(
                "task '{}' cannot be its own parent",
                task.title
            )));
        }

        let mut depends_on = Vec::new();
        let mut seen = HashSet::new();
        for raw_dep in &task.depends_on {
            let dep = resolve_reference(raw_dep, &aliases, existing_ids)?;
            if matches!(dep, ResolvedRef::Local(idx) if idx == task.index) {
                return Err(invalid_spec(format!(
                    "task '{}' cannot depend on itself",
                    task.title
                )));
            }

            let dep_key = match dep {
                ResolvedRef::Local(idx) => (0_u8, idx as u64),
                ResolvedRef::Existing(id) => (1_u8, id),
            };
            if seen.insert(dep_key) {
                depends_on.push(dep);
            }
        }

        resolved.push(ResolvedTaskSpec {
            flat: task,
            parent,
            depends_on,
        });
    }

    Ok(resolved)
}

fn add_order_edge(
    from: usize,
    to: usize,
    outgoing: &mut [Vec<usize>],
    indegree: &mut [usize],
    seen_edges: &mut HashSet<(usize, usize)>,
) {
    if seen_edges.insert((from, to)) {
        outgoing[from].push(to);
        indegree[to] += 1;
    }
}

fn build_creation_order(tasks: &[ResolvedTaskSpec]) -> Result<Vec<usize>> {
    let count = tasks.len();
    let mut outgoing = vec![Vec::new(); count];
    let mut indegree = vec![0_usize; count];
    let mut seen_edges = HashSet::new();

    for task in tasks {
        let to = task.flat.index;
        if let Some(ResolvedRef::Local(from)) = task.parent {
            add_order_edge(from, to, &mut outgoing, &mut indegree, &mut seen_edges);
        }

        for dep in &task.depends_on {
            if let ResolvedRef::Local(from) = *dep {
                add_order_edge(from, to, &mut outgoing, &mut indegree, &mut seen_edges);
            }
        }
    }

    let mut ready: Vec<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(idx, degree)| (*degree == 0).then_some(idx))
        .collect();
    ready.sort_unstable();

    let mut order = Vec::with_capacity(count);
    while let Some(next) = ready.first().copied() {
        ready.remove(0);
        order.push(next);

        for child in &outgoing[next] {
            indegree[*child] -= 1;
            if indegree[*child] == 0 {
                ready.push(*child);
            }
        }
        ready.sort_unstable();
    }

    if order.len() != count {
        let stuck = indegree
            .iter()
            .enumerate()
            .filter_map(|(idx, degree)| {
                (*degree > 0).then_some(task_label_from_resolved(tasks, idx))
            })
            .collect::<Vec<_>>();

        return Err(invalid_spec(format!(
            "cycle detected in imported parent/dependency graph ({})",
            stuck.join(", ")
        )));
    }

    Ok(order)
}

fn resolve_target_id(target: ResolvedRef, created_ids: &HashMap<usize, u64>) -> Result<u64> {
    match target {
        ResolvedRef::Existing(id) => Ok(id),
        ResolvedRef::Local(index) => created_ids.get(&index).copied().ok_or_else(|| {
            invalid_spec(format!(
                "internal ordering error: unresolved local reference '{}'",
                index + 1
            ))
        }),
    }
}

fn apply_import(repo: &Repo, specs: &[ResolvedTaskSpec], order: &[usize]) -> Result<Vec<Task>> {
    let mut created_ids: HashMap<usize, u64> = HashMap::new();
    let mut created: Vec<(usize, Task)> = Vec::with_capacity(order.len());

    for index in order {
        let spec = &specs[*index];

        let parent = spec
            .parent
            .map(|target| resolve_target_id(target, &created_ids))
            .transpose()?;

        let depends_on = spec
            .depends_on
            .iter()
            .copied()
            .map(|target| resolve_target_id(target, &created_ids))
            .collect::<Result<Vec<_>>>()?;

        let (origin_idea_id, refinement_task_ids) =
            derive_traceability(repo, spec.flat.kind, parent, &depends_on)?;

        let mut task = repo.store.create(
            spec.flat.title.clone(),
            spec.flat.kind,
            spec.flat.description.clone(),
            parent,
            depends_on,
            spec.flat.tags.clone(),
            spec.flat.contract.clone(),
            spec.flat.planning.clone(),
        )?;

        let mut traceability_changed = false;
        if origin_idea_id.is_some() {
            task.set_origin_idea_id(origin_idea_id);
            traceability_changed = true;
        }
        if !refinement_task_ids.is_empty() {
            task.set_refinement_task_ids(refinement_task_ids);
            traceability_changed = true;
        }

        if traceability_changed {
            task.normalize();
            repo.store.write(&task)?;
        }

        repo.index.upsert(&task)?;

        created_ids.insert(*index, task.id);
        created.push((*index, task));
    }

    created.sort_by_key(|(index, _)| *index);
    Ok(created.into_iter().map(|(_, task)| task).collect())
}

fn render_reference_for_preview(target: ResolvedRef, tasks: &[ResolvedTaskSpec]) -> String {
    match target {
        ResolvedRef::Existing(id) => TaskId::from(id).to_string(),
        ResolvedRef::Local(index) => tasks[index]
            .flat
            .alias
            .as_ref()
            .map(|alias| format!("@{alias}"))
            .unwrap_or_else(|| format!("#{}", index + 1)),
    }
}

fn build_dry_run_previews(tasks: &[ResolvedTaskSpec], order: &[usize]) -> Vec<DryRunTaskPreview> {
    order
        .iter()
        .enumerate()
        .map(|(position, index)| {
            let task = &tasks[*index];
            DryRunTaskPreview {
                order: position + 1,
                alias: task.flat.alias.clone(),
                title: task.flat.title.clone(),
                kind: task.flat.kind.to_string(),
                parent: task
                    .parent
                    .map(|parent| render_reference_for_preview(parent, tasks)),
                depends_on: task
                    .depends_on
                    .iter()
                    .copied()
                    .map(|dep| render_reference_for_preview(dep, tasks))
                    .collect(),
            }
        })
        .collect()
}

fn print_dry_run_report(
    source: &str,
    tasks: &[ResolvedTaskSpec],
    order: &[usize],
    format: Format,
) -> Result<()> {
    let previews = build_dry_run_previews(tasks, order);

    match format {
        Format::Json => {
            let report = DryRunReport {
                dry_run: true,
                source: source.to_string(),
                task_count: previews.len(),
                tasks: previews,
            };
            println!("{}", serde_json::to_string(&report)?);
        }
        Format::Pretty => {
            println!(
                "{}",
                format!(
                    "Dry run: validated {} tasks from {} (no files written)",
                    previews.len(),
                    source
                )
                .bold()
            );
            for preview in previews {
                let alias = preview
                    .alias
                    .as_deref()
                    .map(|value| format!(" @{value}"))
                    .unwrap_or_default();
                let parent = preview
                    .parent
                    .as_deref()
                    .map(|value| format!(" parent={value}"))
                    .unwrap_or_default();
                let deps = if preview.depends_on.is_empty() {
                    String::new()
                } else {
                    format!(" deps={}", preview.depends_on.join(","))
                };
                println!(
                    "  {:>2}. [{}] {}{}{}{}",
                    preview.order, preview.kind, preview.title, alias, parent, deps,
                );
            }
        }
        Format::Minimal => {
            println!("dry-run {} {}", previews.len(), source);
            for preview in previews {
                println!("{:>2}. {:8} {}", preview.order, preview.kind, preview.title);
            }
        }
    }

    Ok(())
}

fn task_label(tasks: &[FlatTaskSpec], index: usize) -> String {
    tasks[index]
        .alias
        .as_ref()
        .map(|alias| format!("@{alias}"))
        .unwrap_or_else(|| tasks[index].title.clone())
}

fn task_label_from_resolved(tasks: &[ResolvedTaskSpec], index: usize) -> String {
    tasks[index]
        .flat
        .alias
        .as_ref()
        .map(|alias| format!("@{alias}"))
        .unwrap_or_else(|| tasks[index].flat.title.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init;
    use crate::store::repo::Repo;
    use tempfile::tempdir;

    #[test]
    fn flatten_rejects_nested_child_with_explicit_parent() {
        let tasks = vec![ImportTaskSpec {
            id: Some("root".into()),
            title: "Root".into(),
            description: None,
            kind: Kind::Task,
            tags: vec![],
            parent: None,
            depends_on: vec![],
            objective: None,
            acceptance_criteria: vec![],
            verification: vec![],
            constraints: vec![],
            priority: None,
            estimate: None,
            risk: None,
            required_skills: vec![],
            children: vec![ImportTaskSpec {
                id: Some("child".into()),
                title: "Child".into(),
                description: None,
                kind: Kind::Task,
                tags: vec![],
                parent: Some(ImportRef::String("root".into())),
                depends_on: vec![],
                objective: None,
                acceptance_criteria: vec![],
                verification: vec![],
                constraints: vec![],
                priority: None,
                estimate: None,
                risk: None,
                required_skills: vec![],
                children: vec![],
            }],
        }];

        let err = flatten_tasks(tasks).unwrap_err();
        assert!(matches!(err, TakError::ImportInvalidSpec(_)));
    }

    #[test]
    fn run_dry_run_does_not_write_tasks() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
tasks:
  - id: epic
    title: CLI ergonomics follow-up
    kind: epic
    children:
      - id: import
        title: Add tak import command
      - id: tree
        title: Add tak tree sorting
        depends_on: [import]
"#,
        )
        .unwrap();

        run(
            dir.path(),
            import_path.display().to_string(),
            true,
            Format::Json,
        )
        .unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        assert!(repo.store.list_all().unwrap().is_empty());
    }

    #[test]
    fn run_creates_nested_tasks_and_local_dependencies() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
tasks:
  - id: epic
    title: CLI ergonomics follow-up
    kind: epic
    children:
      - id: import
        title: Add tak import command
      - id: tree
        title: Add tak tree sorting
        depends_on: [import]
"#,
        )
        .unwrap();

        run(
            dir.path(),
            import_path.display().to_string(),
            false,
            Format::Json,
        )
        .unwrap();

        let repo = Repo::open(dir.path()).unwrap();
        let tasks = repo.store.list_all().unwrap();
        assert_eq!(tasks.len(), 3);

        let by_title: HashMap<String, Task> = tasks
            .into_iter()
            .map(|task| (task.title.clone(), task))
            .collect();

        let epic = by_title.get("CLI ergonomics follow-up").unwrap();
        let import_task = by_title.get("Add tak import command").unwrap();
        let tree_task = by_title.get("Add tak tree sorting").unwrap();

        assert_eq!(import_task.parent, Some(epic.id));
        assert_eq!(tree_task.parent, Some(epic.id));
        assert_eq!(tree_task.depends_on.len(), 1);
        assert_eq!(tree_task.depends_on[0].id, import_task.id);
    }
}

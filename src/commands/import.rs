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
use crate::store::repo::{resolve_task_id_input, Repo};
use crate::task_id::TaskId;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportPlanSpec {
    epic: String,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    depends_on: Vec<ImportRef>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    verification: Vec<String>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    priority: Option<Priority>,
    #[serde(default)]
    estimate: Option<Estimate>,
    #[serde(default)]
    risk: Option<Risk>,
    #[serde(default)]
    required_skills: Vec<String>,
    #[serde(default)]
    features: Vec<ImportFeatureSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportFeatureSpec {
    #[serde(default)]
    alias: Option<String>,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    depends_on: Vec<ImportRef>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    verification: Vec<String>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    priority: Option<Priority>,
    #[serde(default)]
    estimate: Option<Estimate>,
    #[serde(default)]
    risk: Option<Risk>,
    #[serde(default)]
    required_skills: Vec<String>,
    #[serde(default)]
    tasks: Vec<ImportLeafTaskSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportLeafTaskSpec {
    #[serde(default)]
    alias: Option<String>,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    depends_on: Vec<ImportRef>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    verification: Vec<String>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    priority: Option<Priority>,
    #[serde(default)]
    estimate: Option<Estimate>,
    #[serde(default)]
    risk: Option<Risk>,
    #[serde(default)]
    required_skills: Vec<String>,
}

#[derive(Debug, Clone)]
struct ImportTaskSpec {
    alias: Option<String>,
    title: String,
    description: Option<String>,
    kind: Kind,
    tags: Vec<String>,
    parent: Option<ImportRef>,
    depends_on: Vec<ImportRef>,
    objective: Option<String>,
    acceptance_criteria: Vec<String>,
    verification: Vec<String>,
    constraints: Vec<String>,
    priority: Option<Priority>,
    estimate: Option<Estimate>,
    risk: Option<Risk>,
    required_skills: Vec<String>,
    children: Vec<ImportTaskSpec>,
}

impl ImportPlanSpec {
    fn into_tasks(self) -> Result<Vec<ImportTaskSpec>> {
        if self.features.is_empty() {
            return Err(invalid_spec(
                "import-v2 plan must include at least one feature in `features`",
            ));
        }

        let features = self
            .features
            .into_iter()
            .map(ImportFeatureSpec::into_task)
            .collect::<Result<Vec<_>>>()?;

        Ok(vec![ImportTaskSpec {
            alias: normalize_optional_alias(self.alias),
            title: self.epic,
            description: self.description,
            kind: Kind::Epic,
            tags: self.tags,
            parent: None,
            depends_on: self.depends_on,
            objective: self.objective,
            acceptance_criteria: self.acceptance_criteria,
            verification: self.verification,
            constraints: self.constraints,
            priority: self.priority,
            estimate: self.estimate,
            risk: self.risk,
            required_skills: self.required_skills,
            children: features,
        }])
    }
}

impl ImportFeatureSpec {
    fn into_task(self) -> Result<ImportTaskSpec> {
        let tasks = self
            .tasks
            .into_iter()
            .map(ImportLeafTaskSpec::into_task)
            .collect::<Result<Vec<_>>>()?;

        Ok(ImportTaskSpec {
            alias: normalize_optional_alias(self.alias),
            title: self.title,
            description: self.description,
            kind: Kind::Feature,
            tags: self.tags,
            parent: None,
            depends_on: self.depends_on,
            objective: self.objective,
            acceptance_criteria: self.acceptance_criteria,
            verification: self.verification,
            constraints: self.constraints,
            priority: self.priority,
            estimate: self.estimate,
            risk: self.risk,
            required_skills: self.required_skills,
            children: tasks,
        })
    }
}

impl ImportLeafTaskSpec {
    fn into_task(self) -> Result<ImportTaskSpec> {
        Ok(ImportTaskSpec {
            alias: normalize_optional_alias(self.alias),
            title: self.title,
            description: self.description,
            kind: Kind::Task,
            tags: self.tags,
            parent: None,
            depends_on: self.depends_on,
            objective: self.objective,
            acceptance_criteria: self.acceptance_criteria,
            verification: self.verification,
            constraints: self.constraints,
            priority: self.priority,
            estimate: self.estimate,
            risk: self.risk,
            required_skills: self.required_skills,
            children: Vec::new(),
        })
    }
}

fn normalize_optional_alias(alias: Option<String>) -> Option<String> {
    normalize_optional_text(alias)
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ImportRef {
    String(String),
    Number(u64),
    Node(ImportRefNode),
}

#[derive(Debug, Clone, Deserialize)]
struct ImportRefNode {
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    epic: Option<String>,
}

/// Canonical symbolic reference model used by import-v2 resolution:
/// - `Raw`: scalar YAML input (`"schemas"`, `"0000abcd"`, `42`) resolved as alias first,
///   then existing task id/prefix.
/// - `Alias`: mapping/anchor-derived reference with explicit `alias` field; must match an
///   imported alias.
/// - `Title`: mapping/anchor-derived reference without alias; must match exactly one imported
///   task title in this plan, otherwise it is rejected as unresolved/ambiguous.
#[derive(Debug, Clone)]
enum NormalizedRef {
    Raw(String),
    Alias(String),
    Title(String),
}

impl NormalizedRef {
    fn display(&self) -> String {
        match self {
            Self::Raw(value) => value.clone(),
            Self::Alias(value) => format!("@{value}"),
            Self::Title(value) => format!("title:{value}"),
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
    explicit_parent: Option<NormalizedRef>,
    nested_parent: Option<usize>,
    depends_on: Vec<NormalizedRef>,
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
    depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    title: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    estimate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    risk: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    required_skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    objective: Option<String>,
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

    let spec: ImportPlanSpec = serde_yaml::from_str(raw).map_err(|yaml_err| {
        invalid_spec(format!(
            "failed to parse '{source}' as import-v2 YAML plan: {yaml_err}"
        ))
    })?;

    spec.into_tasks()
}

fn normalize_reference(reference: &ImportRef) -> Result<NormalizedRef> {
    match reference {
        ImportRef::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(invalid_spec("reference values cannot be empty"));
            }
            Ok(NormalizedRef::Raw(trimmed.to_string()))
        }
        ImportRef::Number(value) => Ok(NormalizedRef::Raw(value.to_string())),
        ImportRef::Node(node) => {
            if let Some(alias) = normalize_optional_text(node.alias.clone()) {
                return Ok(NormalizedRef::Alias(alias));
            }
            if let Some(title) = normalize_optional_text(node.title.clone()) {
                return Ok(NormalizedRef::Title(title));
            }
            if let Some(epic) = normalize_optional_text(node.epic.clone()) {
                return Ok(NormalizedRef::Title(epic));
            }
            Err(invalid_spec(
                "mapping references must include non-empty `alias`, `title`, or `epic`",
            ))
        }
    }
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

    let alias = task.alias.and_then(|value| {
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

fn build_title_map(tasks: &[FlatTaskSpec]) -> HashMap<String, Vec<usize>> {
    let mut titles: HashMap<String, Vec<usize>> = HashMap::new();

    for task in tasks {
        titles
            .entry(task.title.clone())
            .or_default()
            .push(task.index);
    }

    titles
}

fn resolve_reference(
    reference: &NormalizedRef,
    aliases: &HashMap<String, usize>,
    titles: &HashMap<String, Vec<usize>>,
    existing_ids: &[TaskId],
) -> Result<ResolvedRef> {
    match reference {
        NormalizedRef::Alias(alias) => aliases
            .get(alias)
            .copied()
            .map(ResolvedRef::Local)
            .ok_or_else(|| {
                invalid_spec(format!(
                    "symbolic reference '{}' does not match an imported alias",
                    reference.display()
                ))
            }),
        NormalizedRef::Title(title) => {
            let Some(matches) = titles.get(title) else {
                return Err(invalid_spec(format!(
                    "symbolic reference '{}' does not match any imported task title",
                    reference.display()
                )));
            };

            if matches.len() > 1 {
                return Err(invalid_spec(format!(
                    "symbolic reference '{}' is ambiguous ({} tasks share this title); add explicit `alias` fields and reference those aliases instead",
                    reference.display(),
                    matches.len()
                )));
            }

            Ok(ResolvedRef::Local(matches[0]))
        }
        NormalizedRef::Raw(raw) => {
            if let Some(index) = aliases.get(raw).copied() {
                return Ok(ResolvedRef::Local(index));
            }

            let resolved = resolve_task_id_input(raw, existing_ids).map_err(|err| {
                invalid_spec(format!(
                    "reference '{}' does not match an imported alias or existing task id: {err}",
                    reference.display()
                ))
            })?;

            Ok(ResolvedRef::Existing(resolved.into()))
        }
    }
}

fn resolve_tasks(
    flat: Vec<FlatTaskSpec>,
    existing_ids: &[TaskId],
) -> Result<Vec<ResolvedTaskSpec>> {
    let aliases = build_alias_map(&flat)?;
    let titles = build_title_map(&flat);
    let mut resolved = Vec::with_capacity(flat.len());

    for task in flat {
        let parent = if let Some(parent_index) = task.nested_parent {
            Some(ResolvedRef::Local(parent_index))
        } else if let Some(raw_parent) = task.explicit_parent.as_ref() {
            Some(resolve_reference(
                raw_parent,
                &aliases,
                &titles,
                existing_ids,
            )?)
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
            let dep = resolve_reference(raw_dep, &aliases, &titles, existing_ids)?;
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

fn rollback_import(repo: &Repo, created_task_ids: &[u64]) -> Result<()> {
    let mut cleanup_failures = Vec::new();

    for id in created_task_ids.iter().rev() {
        if let Err(err) = repo.index.remove(*id) {
            cleanup_failures.push(format!("index remove {} failed: {err}", TaskId::from(*id)));
        }

        if let Err(err) = repo.store.delete(*id) {
            if !matches!(err, TakError::TaskNotFound(_)) {
                cleanup_failures.push(format!(
                    "task file delete {} failed: {err}",
                    TaskId::from(*id)
                ));
            }
        }
    }

    if cleanup_failures.is_empty() {
        Ok(())
    } else {
        Err(invalid_spec(format!(
            "import rollback failed after partial apply: {}",
            cleanup_failures.join("; ")
        )))
    }
}

fn abort_import_with_rollback<T>(
    repo: &Repo,
    created_task_ids: &[u64],
    err: TakError,
) -> Result<T> {
    match rollback_import(repo, created_task_ids) {
        Ok(_) => Err(err),
        Err(rollback_err) => Err(invalid_spec(format!(
            "import apply failed ({err}); rollback failed ({rollback_err})"
        ))),
    }
}

#[cfg(not(test))]
fn maybe_inject_apply_failure(_created_count: usize) -> Result<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static APPLY_FAIL_AFTER_CREATE: std::cell::Cell<Option<usize>> = std::cell::Cell::new(None);
}

#[cfg(test)]
struct ApplyFailAfterCreateGuard {
    previous: Option<usize>,
}

#[cfg(test)]
impl Drop for ApplyFailAfterCreateGuard {
    fn drop(&mut self) {
        APPLY_FAIL_AFTER_CREATE.with(|cell| cell.set(self.previous));
    }
}

#[cfg(test)]
fn set_apply_fail_after_create_for_test(value: Option<usize>) -> ApplyFailAfterCreateGuard {
    let previous = APPLY_FAIL_AFTER_CREATE.with(|cell| {
        let prev = cell.get();
        cell.set(value);
        prev
    });

    ApplyFailAfterCreateGuard { previous }
}

#[cfg(test)]
fn maybe_inject_apply_failure(created_count: usize) -> Result<()> {
    let should_fail = APPLY_FAIL_AFTER_CREATE.with(|cell| {
        cell.get()
            .is_some_and(|threshold| created_count >= threshold)
    });

    if should_fail {
        Err(invalid_spec(format!(
            "injected import apply failure after creating {created_count} task(s)"
        )))
    } else {
        Ok(())
    }
}

fn apply_import(repo: &Repo, specs: &[ResolvedTaskSpec], order: &[usize]) -> Result<Vec<Task>> {
    let mut created_ids: HashMap<usize, u64> = HashMap::new();
    let mut created: Vec<(usize, Task)> = Vec::with_capacity(order.len());
    let mut created_task_ids: Vec<u64> = Vec::with_capacity(order.len());

    for index in order {
        let spec = &specs[*index];

        let parent = match spec
            .parent
            .map(|target| resolve_target_id(target, &created_ids))
            .transpose()
        {
            Ok(parent) => parent,
            Err(err) => return abort_import_with_rollback(repo, &created_task_ids, err),
        };

        let depends_on = match spec
            .depends_on
            .iter()
            .copied()
            .map(|target| resolve_target_id(target, &created_ids))
            .collect::<Result<Vec<_>>>()
        {
            Ok(depends_on) => depends_on,
            Err(err) => return abort_import_with_rollback(repo, &created_task_ids, err),
        };

        let (origin_idea_id, refinement_task_ids) =
            match derive_traceability(repo, spec.flat.kind, parent, &depends_on) {
                Ok(traceability) => traceability,
                Err(err) => return abort_import_with_rollback(repo, &created_task_ids, err),
            };

        let mut task = match repo.store.create(
            spec.flat.title.clone(),
            spec.flat.kind,
            spec.flat.description.clone(),
            parent,
            depends_on,
            spec.flat.tags.clone(),
            spec.flat.contract.clone(),
            spec.flat.planning.clone(),
        ) {
            Ok(task) => task,
            Err(err) => return abort_import_with_rollback(repo, &created_task_ids, err),
        };

        created_task_ids.push(task.id);

        if let Err(err) = maybe_inject_apply_failure(created_task_ids.len()) {
            return abort_import_with_rollback(repo, &created_task_ids, err);
        }

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
            if let Err(err) = repo.store.write(&task) {
                return abort_import_with_rollback(repo, &created_task_ids, err);
            }
        }

        if let Err(err) = repo.index.upsert(&task) {
            return abort_import_with_rollback(repo, &created_task_ids, err);
        }

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

fn preview_depth(index: usize, tasks: &[ResolvedTaskSpec]) -> usize {
    let mut depth = 0;
    let mut current = index;

    while let Some(ResolvedRef::Local(parent)) = tasks[current].parent {
        depth += 1;
        current = parent;
        if depth > tasks.len() {
            break;
        }
    }

    depth
}

fn truncate_preview_text(value: &str, max_chars: usize) -> String {
    let mut iter = value.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn preview_metadata_segments(preview: &DryRunTaskPreview) -> Vec<String> {
    let mut segments = Vec::new();

    if let Some(priority) = preview.priority.as_deref() {
        segments.push(format!("p={priority}"));
    }
    if let Some(estimate) = preview.estimate.as_deref() {
        segments.push(format!("est={estimate}"));
    }
    if let Some(risk) = preview.risk.as_deref() {
        segments.push(format!("risk={risk}"));
    }
    if !preview.tags.is_empty() {
        segments.push(format!("tags={}", preview.tags.join(",")));
    }
    if !preview.required_skills.is_empty() {
        segments.push(format!("skills={}", preview.required_skills.join(",")));
    }
    if let Some(objective) = preview.objective.as_deref() {
        segments.push(format!(
            "objective={}",
            truncate_preview_text(objective, 40)
        ));
    }

    segments
}

fn build_dry_run_previews(tasks: &[ResolvedTaskSpec], order: &[usize]) -> Vec<DryRunTaskPreview> {
    order
        .iter()
        .enumerate()
        .map(|(position, index)| {
            let task = &tasks[*index];
            DryRunTaskPreview {
                order: position + 1,
                depth: preview_depth(*index, tasks),
                alias: task.flat.alias.clone(),
                title: task.flat.title.clone(),
                kind: task.flat.kind.to_string(),
                description: task.flat.description.clone(),
                parent: task
                    .parent
                    .map(|parent| render_reference_for_preview(parent, tasks)),
                depends_on: task
                    .depends_on
                    .iter()
                    .copied()
                    .map(|dep| render_reference_for_preview(dep, tasks))
                    .collect(),
                tags: task.flat.tags.clone(),
                priority: task.flat.planning.priority.map(|value| value.to_string()),
                estimate: task.flat.planning.estimate.map(|value| value.to_string()),
                risk: task.flat.planning.risk.map(|value| value.to_string()),
                required_skills: task.flat.planning.required_skills.clone(),
                objective: task.flat.contract.objective.clone(),
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
                let description = preview
                    .description
                    .as_deref()
                    .map(|value| format!(" desc={}", truncate_preview_text(value, 48)))
                    .unwrap_or_default();
                let metadata = preview_metadata_segments(&preview);
                let metadata = if metadata.is_empty() {
                    String::new()
                } else {
                    format!(" {}", metadata.join(" "))
                };
                let indent = "  ".repeat(preview.depth);
                println!(
                    "  {:>2}. {}[{}] {}{}{}{}{}{}",
                    preview.order,
                    indent,
                    preview.kind,
                    preview.title,
                    alias,
                    parent,
                    deps,
                    description,
                    metadata,
                );
            }
        }
        Format::Minimal => {
            println!("dry-run {} {}", previews.len(), source);
            for preview in previews {
                let mut compact_meta = Vec::new();
                if let Some(priority) = preview.priority.as_deref() {
                    compact_meta.push(format!("p={priority}"));
                }
                if let Some(estimate) = preview.estimate.as_deref() {
                    compact_meta.push(format!("est={estimate}"));
                }
                if !preview.tags.is_empty() {
                    compact_meta.push(format!("tags={}", preview.tags.len()));
                }
                let compact_meta = if compact_meta.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", compact_meta.join(" "))
                };
                let indent = "  ".repeat(preview.depth);
                println!(
                    "{:>2}. {:8} {}{}{}",
                    preview.order, preview.kind, indent, preview.title, compact_meta
                );
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

    fn build_previews_for_raw(raw: &str) -> Vec<DryRunTaskPreview> {
        let parsed = parse_document("preview.yaml", raw).unwrap();
        let flat = flatten_tasks(parsed).unwrap();
        let resolved = resolve_tasks(flat, &[]).unwrap();
        let order = build_creation_order(&resolved).unwrap();
        build_dry_run_previews(&resolved, &order)
    }

    #[test]
    fn parse_document_rejects_legacy_payload_shape() {
        let raw = r#"
tasks:
  - title: Legacy import payload
"#;

        let err = parse_document("legacy.yaml", raw).unwrap_err();
        let TakError::ImportInvalidSpec(message) = err else {
            panic!("expected import invalid spec error");
        };
        assert!(message.contains("failed to parse"));
    }

    #[test]
    fn parse_document_rejects_unknown_fields() {
        let raw = r#"
epic: Import v2
oops: true
features:
  - title: Feature A
"#;

        let err = parse_document("unknown.yaml", raw).unwrap_err();
        let TakError::ImportInvalidSpec(message) = err else {
            panic!("expected import invalid spec error");
        };
        assert!(message.contains("unknown field"));
        assert!(message.contains("oops"));
    }

    #[test]
    fn parse_document_requires_at_least_one_feature() {
        let raw = r#"
epic: Import v2
features: []
"#;

        let err = parse_document("missing-features.yaml", raw).unwrap_err();
        let TakError::ImportInvalidSpec(message) = err else {
            panic!("expected import invalid spec error");
        };
        assert!(message.contains("at least one feature"));
    }

    #[test]
    fn run_dry_run_does_not_write_tasks() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
epic: CLI ergonomics follow-up
features:
  - alias: import
    title: Add tak import command
    tasks:
      - alias: tree
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
epic: CLI ergonomics follow-up
features:
  - alias: import
    title: Add tak import command
    tasks:
      - alias: tree
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
        let feature_task = by_title.get("Add tak import command").unwrap();
        let tree_task = by_title.get("Add tak tree sorting").unwrap();

        assert_eq!(feature_task.parent, Some(epic.id));
        assert_eq!(tree_task.parent, Some(feature_task.id));
        assert_eq!(tree_task.depends_on.len(), 1);
        assert_eq!(tree_task.depends_on[0].id, feature_task.id);
    }

    #[test]
    fn run_resolves_cross_feature_symbolic_dependencies_from_yaml_anchors() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
epic: Agentic Chat
features:
  - &infra
    title: Tool Infrastructure
    tasks:
      - &schemas
        title: Define tool schemas
      - title: Add agentMode flag
        depends_on: [*schemas]
  - &read
    title: Read Tools
    depends_on: [*infra]
    tasks:
      - title: Wire read handlers
  - title: Write Tools
    depends_on: [*read]
    tasks:
      - title: Approval gate UI
      - title: Edit scene tool
        depends_on: [*schemas, *infra]
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
        let by_title: HashMap<String, Task> = tasks
            .iter()
            .cloned()
            .map(|task| (task.title.clone(), task))
            .collect();
        let id_to_title: HashMap<u64, String> = tasks
            .iter()
            .map(|task| (task.id, task.title.clone()))
            .collect();

        let add_agent_mode = by_title.get("Add agentMode flag").unwrap();
        let read_tools = by_title.get("Read Tools").unwrap();
        let write_tools = by_title.get("Write Tools").unwrap();
        let edit_scene = by_title.get("Edit scene tool").unwrap();

        let add_agent_mode_deps: Vec<String> = add_agent_mode
            .depends_on
            .iter()
            .map(|dep| id_to_title.get(&dep.id).unwrap().clone())
            .collect();
        assert_eq!(add_agent_mode_deps, vec!["Define tool schemas"]);

        let read_tools_deps: Vec<String> = read_tools
            .depends_on
            .iter()
            .map(|dep| id_to_title.get(&dep.id).unwrap().clone())
            .collect();
        assert_eq!(read_tools_deps, vec!["Tool Infrastructure"]);

        let write_tools_deps: Vec<String> = write_tools
            .depends_on
            .iter()
            .map(|dep| id_to_title.get(&dep.id).unwrap().clone())
            .collect();
        assert_eq!(write_tools_deps, vec!["Read Tools"]);

        let mut edit_scene_deps: Vec<String> = edit_scene
            .depends_on
            .iter()
            .map(|dep| id_to_title.get(&dep.id).unwrap().clone())
            .collect();
        edit_scene_deps.sort();
        assert_eq!(
            edit_scene_deps,
            vec!["Define tool schemas", "Tool Infrastructure"]
        );
    }

    #[test]
    fn run_rejects_ambiguous_title_symbolic_references() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
epic: Ambiguous refs
features:
  - title: Feature A
    tasks:
      - &shared_a
        title: Shared Task
  - title: Feature B
    tasks:
      - &shared_b
        title: Shared Task
      - title: Consumer
        depends_on: [*shared_a]
"#,
        )
        .unwrap();

        let err = run(
            dir.path(),
            import_path.display().to_string(),
            true,
            Format::Json,
        )
        .unwrap_err();

        let TakError::ImportInvalidSpec(message) = err else {
            panic!("expected import invalid spec error");
        };
        assert!(message.contains("ambiguous"));
        assert!(message.contains("title:Shared Task"));
    }

    #[test]
    fn run_rejects_unresolved_symbolic_title_references() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
epic: Missing refs
features:
  - title: Feature A
    tasks:
      - title: Consumer
        depends_on:
          - title: Missing Task
"#,
        )
        .unwrap();

        let err = run(
            dir.path(),
            import_path.display().to_string(),
            true,
            Format::Json,
        )
        .unwrap_err();

        let TakError::ImportInvalidSpec(message) = err else {
            panic!("expected import invalid spec error");
        };
        assert!(message.contains("does not match any imported task title"));
        assert!(message.contains("title:Missing Task"));
    }

    #[test]
    fn run_preserves_metadata_across_epic_feature_and_task() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
epic: Agentic Chat
description: Epic description
tags: [agentic-chat, planning]
priority: high
estimate: l
risk: medium
required_skills: [planning]
objective: Epic objective
acceptance_criteria: [Epic criterion]
verification: [echo epic]
constraints: [epic constraint]
features:
  - title: Tool Infrastructure
    description: Feature description
    tags: [backend]
    priority: high
    estimate: m
    risk: high
    required_skills: [rust]
    objective: Feature objective
    acceptance_criteria: [Feature criterion]
    verification: [echo feature]
    constraints: [feature constraint]
    tasks:
      - title: Define tool schemas
        description: Task description
        tags: [backend, schema]
        priority: medium
        estimate: s
        risk: low
        required_skills: [serde]
        objective: Task objective
        acceptance_criteria: [Task criterion]
        verification: [echo task]
        constraints: [task constraint]
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
        let by_title: HashMap<String, Task> = tasks
            .into_iter()
            .map(|task| (task.title.clone(), task))
            .collect();

        let epic = by_title.get("Agentic Chat").unwrap();
        assert_eq!(epic.kind, Kind::Epic);
        assert_eq!(epic.description.as_deref(), Some("Epic description"));
        assert_eq!(epic.tags, vec!["agentic-chat", "planning"]);
        assert_eq!(epic.planning.priority, Some(Priority::High));
        assert_eq!(epic.planning.estimate, Some(Estimate::L));
        assert_eq!(epic.planning.risk, Some(Risk::Medium));
        assert_eq!(epic.planning.required_skills, vec!["planning"]);
        assert_eq!(epic.contract.objective.as_deref(), Some("Epic objective"));
        assert_eq!(epic.contract.acceptance_criteria, vec!["Epic criterion"]);
        assert_eq!(epic.contract.verification, vec!["echo epic"]);
        assert_eq!(epic.contract.constraints, vec!["epic constraint"]);

        let feature = by_title.get("Tool Infrastructure").unwrap();
        assert_eq!(feature.kind, Kind::Feature);
        assert_eq!(feature.description.as_deref(), Some("Feature description"));
        assert_eq!(feature.tags, vec!["backend"]);
        assert_eq!(feature.planning.priority, Some(Priority::High));
        assert_eq!(feature.planning.estimate, Some(Estimate::M));
        assert_eq!(feature.planning.risk, Some(Risk::High));
        assert_eq!(feature.planning.required_skills, vec!["rust"]);
        assert_eq!(
            feature.contract.objective.as_deref(),
            Some("Feature objective")
        );
        assert_eq!(
            feature.contract.acceptance_criteria,
            vec!["Feature criterion"]
        );
        assert_eq!(feature.contract.verification, vec!["echo feature"]);
        assert_eq!(feature.contract.constraints, vec!["feature constraint"]);

        let task = by_title.get("Define tool schemas").unwrap();
        assert_eq!(task.kind, Kind::Task);
        assert_eq!(task.description.as_deref(), Some("Task description"));
        assert_eq!(task.tags, vec!["backend", "schema"]);
        assert_eq!(task.planning.priority, Some(Priority::Medium));
        assert_eq!(task.planning.estimate, Some(Estimate::S));
        assert_eq!(task.planning.risk, Some(Risk::Low));
        assert_eq!(task.planning.required_skills, vec!["serde"]);
        assert_eq!(task.contract.objective.as_deref(), Some("Task objective"));
        assert_eq!(task.contract.acceptance_criteria, vec!["Task criterion"]);
        assert_eq!(task.contract.verification, vec!["echo task"]);
        assert_eq!(task.contract.constraints, vec!["task constraint"]);
    }

    #[test]
    fn dry_run_previews_include_hierarchy_and_key_metadata() {
        let previews = build_previews_for_raw(
            r#"
epic: Agentic Chat
description: Epic description
tags: [agentic-chat]
priority: high
estimate: l
features:
  - title: Tool Infrastructure
    tags: [backend]
    priority: medium
    tasks:
      - title: Define tool schemas
        tags: [backend, schema]
        priority: low
        estimate: s
        risk: low
        required_skills: [serde]
        objective: Task objective
"#,
        );

        let by_title: HashMap<String, DryRunTaskPreview> = previews
            .into_iter()
            .map(|preview| (preview.title.clone(), preview))
            .collect();

        let epic = by_title.get("Agentic Chat").unwrap();
        assert_eq!(epic.depth, 0);
        assert_eq!(epic.priority.as_deref(), Some("high"));
        assert_eq!(epic.estimate.as_deref(), Some("l"));
        assert_eq!(epic.tags, vec!["agentic-chat"]);

        let feature = by_title.get("Tool Infrastructure").unwrap();
        assert_eq!(feature.depth, 1);
        assert_eq!(feature.priority.as_deref(), Some("medium"));
        assert_eq!(feature.tags, vec!["backend"]);

        let task = by_title.get("Define tool schemas").unwrap();
        assert_eq!(task.depth, 2);
        assert_eq!(task.priority.as_deref(), Some("low"));
        assert_eq!(task.estimate.as_deref(), Some("s"));
        assert_eq!(task.risk.as_deref(), Some("low"));
        assert_eq!(task.tags, vec!["backend", "schema"]);
        assert_eq!(task.required_skills, vec!["serde"]);
        assert_eq!(task.objective.as_deref(), Some("Task objective"));
    }

    #[test]
    fn run_rolls_back_partial_apply_when_a_create_step_fails() {
        let dir = tempdir().unwrap();
        init::run(dir.path()).unwrap();

        let import_path = dir.path().join("import.yaml");
        fs::write(
            &import_path,
            r#"
epic: Rollback check
features:
  - title: Feature A
    tasks:
      - title: Task one
      - title: Task two
"#,
        )
        .unwrap();

        let _guard = set_apply_fail_after_create_for_test(Some(2));

        let err = run(
            dir.path(),
            import_path.display().to_string(),
            false,
            Format::Json,
        )
        .unwrap_err();

        let TakError::ImportInvalidSpec(message) = err else {
            panic!("expected import invalid spec error");
        };
        assert!(message.contains("injected import apply failure"));

        let repo = Repo::open(dir.path()).unwrap();
        assert!(repo.store.list_all().unwrap().is_empty());
        assert!(repo.index.roots().unwrap().is_empty());
    }
}

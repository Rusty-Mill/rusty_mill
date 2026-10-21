"""Stage 1 composition: CI graph + Millboard registry + RemindMe snapshots.

No repository-supplied code is imported. Snapshots are data, never commands.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

from affected_crates import reverse_dependencies
from check_workspace_layers import workspace_packages
from repository_intelligence import analyze, closure, main as inspection_main


def digest(value: object) -> str:
    """Content identity includes scope, lifecycle and revision, not just prose."""
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                     ensure_ascii=False).encode()).hexdigest()


def cargo_metadata_digest(metadata: dict) -> str:
    """Hash Cargo identity after canonicalizing only schema-defined sets.

    Cargo does not promise an order for these collections, and Windows has
    emitted consecutive equivalent graphs in different orders. Package and
    workspace ``metadata`` values are deliberately opaque here: in particular,
    lists inside application-defined metadata retain their authored order.
    """
    normalized = canonical_cargo_metadata(metadata)

    return digest(normalized)


def canonical_cargo_metadata(metadata: dict) -> dict:
    """Copy metadata and sort only collections Cargo defines as unordered."""
    normalized = copy.deepcopy(metadata)

    def windows_path(value: object) -> object:
        if not isinstance(value, str):
            return value
        # Win32 normally trims trailing periods and spaces from components,
        # while an extended-length path addresses those components literally.
        # Removing the prefix is therefore safe only when every component has
        # the same meaning under both spellings.
        components = value.split('\\')
        if any(component.endswith(('.', ' ')) for component in components if component):
            return value
        if re.match(r'^\\\\\?\\UNC\\[^\\]+\\[^\\]+(?:\\|$)', value, re.IGNORECASE):
            return '\\\\' + value[8:]
        if re.match(r'^\\\\\?\\[A-Za-z]:\\', value):
            return value[4:]
        return value

    def path_field(value: dict, field: str) -> None:
        if field in value:
            value[field] = windows_path(value[field])

    for field in ('workspace_root', 'target_directory', 'build_directory'):
        path_field(normalized, field)
    for package in normalized.get('packages', []):
        path_field(package, 'manifest_path')
        for dependency in package.get('dependencies', []):
            path_field(dependency, 'path')
        for target in package.get('targets', []):
            path_field(target, 'src_path')
    def canonical(value: object) -> str:
        return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)

    def sort_field(value: dict, field: str) -> None:
        if isinstance(value.get(field), list):
            value[field].sort(key=canonical)

    sort_field(normalized, 'workspace_members')
    sort_field(normalized, 'workspace_default_members')
    for package in normalized.get('packages', []):
        for dependency in package.get('dependencies', []):
            sort_field(dependency, 'features')
        for target in package.get('targets', []):
            sort_field(target, 'kind')
            sort_field(target, 'crate_types')
            sort_field(target, 'required-features')
        sort_field(package, 'dependencies')
        sort_field(package, 'targets')
        for enabled_by in package.get('features', {}).values():
            if isinstance(enabled_by, list):
                enabled_by.sort(key=canonical)
    sort_field(normalized, 'packages')
    resolve = normalized.get('resolve')
    if isinstance(resolve, dict):
        for node in resolve.get('nodes', []):
            sort_field(node, 'dependencies')
            sort_field(node, 'features')
            for dependency in node.get('deps', []):
                sort_field(dependency, 'dep_kinds')
            sort_field(node, 'deps')
        sort_field(resolve, 'nodes')
    return normalized


def cargo_metadata_diagnostics(metadata: dict) -> dict:
    """Return schema-aware, bounded fingerprints without authored content."""
    max_records, max_depth, max_bytes = 128, 12, 8192
    normalized = canonical_cargo_metadata(metadata)
    records = []
    omitted = {'records': 0, 'depth': 0, 'bytes': 0}

    def kind(value: object) -> str:
        if value is None:
            return 'null'
        if isinstance(value, bool):
            return 'bool'
        if isinstance(value, (int, float)):
            return 'number'
        if isinstance(value, str):
            return 'string'
        if isinstance(value, list):
            return 'array'
        return 'object'

    def add(pointer: str, value: object, **extra: str) -> None:
        if len(records) == max_records:
            omitted['records'] += 1
            return
        records.append({'pointer': pointer or '/', 'type': kind(value),
                        'fingerprint': digest(value)[:16], **extra})

    path_fields = {'manifest_path', 'src_path', 'workspace_root', 'target_directory',
                   'build_directory'}

    def visit(value: object, pointer: str, depth: int, field: str = '') -> None:
        if depth > max_depth:
            omitted['depth'] += 1
            return
        # Cargo declares package/workspace metadata opaque. Fingerprint the
        # canonical subtree as one unit so authored keys and values never leak.
        if field == 'metadata':
            add(pointer, value, opaque='authored-metadata')
        elif isinstance(value, dict):
            if not value:
                add(pointer, value)
            for key in sorted(value):
                escaped = key.replace('~', '~0').replace('/', '~1')
                visit(value[key], f'{pointer}/{escaped}', depth + 1, key)
        elif isinstance(value, list):
            if not value:
                add(pointer, value)
            for index, child in enumerate(value):
                visit(child, f'{pointer}/{index}', depth + 1)
        else:
            extra = {}
            dependency_path = field == 'path' and '/dependencies/' in pointer
            if isinstance(value, str) and (field in path_fields or dependency_path):
                extra['path_spelling'] = (
                    'extended-unc' if value.startswith('\\\\?\\UNC\\') else
                    'extended' if value.startswith('\\\\?\\') else 'ordinary')
            add(pointer, value, **extra)

    candidate_paths = {
        'workspace_root': [normalized.get('workspace_root')],
        'target_directory': [normalized.get('target_directory')],
        'build_directory': [normalized.get('build_directory')],
        'manifest_path': [package.get('manifest_path')
                          for package in normalized.get('packages', [])],
        'dependency.path': [dependency.get('path')
                            for package in normalized.get('packages', [])
                            for dependency in package.get('dependencies', [])],
        'targets.src_path': [target.get('src_path')
                             for package in normalized.get('packages', [])
                             for target in package.get('targets', [])],
    }

    def path_summary(values: list) -> dict:
        values = [value for value in values if isinstance(value, str)]
        spellings = {'extended': 0, 'extended-unc': 0, 'ordinary': 0}
        for value in values:
            spelling = ('extended-unc' if value.startswith('\\\\?\\UNC\\') else
                        'extended' if value.startswith('\\\\?\\') else 'ordinary')
            spellings[spelling] += 1
        return {'count': len(values), 'fingerprint': digest(values)[:16],
                'spellings': spellings}

    known_top_level = ('packages', 'workspace_members', 'workspace_default_members',
                       'resolve', 'target_directory', 'build_directory',
                       'workspace_root', 'version', 'metadata')
    top_level = {field: digest(normalized[field])[:16]
                 for field in known_top_level if field in normalized}
    packages = normalized.get('packages', [])
    package_limit = 32
    package_fingerprints = [digest(package)[:16] for package in packages[:package_limit]]
    package_remainder = (digest(packages[package_limit:])[:16]
                         if len(packages) > package_limit else None)
    visit(normalized, '', 0)
    result = {
        'schema_version': 2,
        'top_level_fingerprints': top_level,
        'package_fingerprints': package_fingerprints,
        'package_remainder': {'count': max(0, len(packages) - package_limit),
                              'fingerprint': package_remainder},
        'path_fields': {field: path_summary(values)
                        for field, values in candidate_paths.items()},
        'records': records,
        'truncated': omitted,
    }
    encoded = lambda: json.dumps(result, sort_keys=True, separators=(',', ':')).encode()
    while len(encoded()) > max_bytes and records:
        records.pop()
        omitted['bytes'] += 1
    return result


def relative_path(value: str) -> str:
    """Require portable repository-relative paths, including deleted paths."""
    if not isinstance(value, str) or not value or value.startswith('/') or '\\' in value or ':' in value:
        raise ValueError("expected a repository-relative path")
    value = value.rstrip('/')
    if any(p in ('', '.', '..') for p in value.split('/')):
        raise ValueError("path must stay inside the repository")
    return value


def contains(parent: str, child: str) -> bool:
    return child == parent or child.startswith(parent + '/')


def strings(value: object) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(v, str) or not v for v in value):
        raise ValueError("expected a list of nonempty strings")
    return value


def registry(value: dict, repository: str) -> dict:
    """Read Millboard file_json / board_projects shapes and preserve its owner rule.

    Millboard selects the first project in name order with a containing path,
    then the repository default. It does not use longest-prefix ownership.
    """
    repos = value.get('repos', {'default': value})
    if not isinstance(repos, dict) or ('repos' in value and set(value) != {'repos'}):
        raise ValueError("invalid Millboard repository registry")
    if repository not in repos:
        raise ValueError("repository is missing from Millboard projects")
    seen, result = set(), {}
    for repo, projects in sorted(repos.items()):
        if not re.fullmatch(r'[a-z0-9_-]{1,40}', repo) or repo == 'repos' or not isinstance(projects, dict):
            raise ValueError("invalid Millboard projects")
        if repo in seen:
            raise ValueError("duplicate Millboard repository default")
        seen.add(repo)
        paths_seen = set()
        for name, paths in sorted(projects.items()):
            if not re.fullmatch(r'[a-z0-9_-]{1,40}', name) or name == 'repos' or (name != repo and name in seen):
                raise ValueError("invalid or duplicate Millboard project")
            seen.add(name)
            cleaned = [relative_path(p) for p in strings(paths)]
            if name == repo and cleaned:
                raise ValueError("repository default cannot have paths")
            if name != repo and not cleaned:
                raise ValueError("project needs paths")
            for path in cleaned:
                if path in paths_seen:
                    raise ValueError("duplicate Millboard ownership path")
                paths_seen.add(path)
            if repo == repository:
                result[name] = cleaned
    result.setdefault(repository, [])
    return result


def owner(projects: dict, repository: str, path: str) -> str:
    return next((name for name, paths in sorted(projects.items())
                 if any(contains(p, path) for p in paths)), repository)


def requirements(snapshot: dict) -> list[dict]:
    """A ranked result cannot substitute for the complete durable inventory."""
    if snapshot.get('schema_version') != 1 or snapshot.get('source') != 'remind_me' or snapshot.get('complete') is not True:
        raise ValueError("complete RemindMe impact-export snapshot required")
    records = snapshot['records']
    if not isinstance(records, list):
        raise ValueError("invalid requirement inventory")
    result, seen = [], set()
    for memory in records:
        scope = memory.get('metadata', {}).get('factory_requirement')
        if 'factory_requirement' not in memory.get('metadata', {}):
            continue
        if not isinstance(scope, dict) or not isinstance(scope.get('binding'), bool):
            raise ValueError("factory_requirement needs an explicit binding boolean")
        key = memory['id']
        if not isinstance(key, str) or not key or key in seen:
            raise ValueError("duplicate or invalid requirement id")
        seen.add(key)
        if not isinstance(memory.get('updated_at'), str) or not memory['updated_at']:
            raise ValueError("requirement needs updated_at revision")
        if not isinstance(memory.get('content'), str) or not isinstance(scope.get('repository'), str):
            raise ValueError("requirement needs content and repository")
        for field in ('projects', 'packages', 'paths', 'validation'):
            strings(scope.get(field, []))
        for path in scope.get('paths', []):
            relative_path(path)
        # Retrieval access counts, vitality and rank are not contract revisions.
        contract = {k: memory.get(k) for k in ('id', 'updated_at', 'content', 'metadata',
                    'status', 'deleted_at', 'superseded_by', 'valid_from', 'valid_until', 'outcome')}
        result.append({'id': key, 'revision': memory['updated_at'], 'fingerprint': digest(contract),
                       'scope': scope, 'content': memory['content'],
                       'lifecycle': {k: memory.get(k) for k in ('status', 'deleted_at', 'superseded_by', 'valid_until')}})
    return sorted(result, key=lambda r: r['id'])


def analyze_impact(metadata: dict, request: dict, projects_data: dict, snapshot: dict,
                   source_revision: str, previous: dict | None = None) -> dict:
    """Join existing intelligence by package identity, never by repository name."""
    if request.get('schema_version') != 1:
        raise ValueError("unsupported impact request schema")
    intelligence = analyze(metadata)
    root = Path(intelligence['workspace_root'])
    repository = request['repository']
    projects = registry(projects_data, repository)
    crates = {c['id']: c for c in intelligence['crates']}
    by_name = {}
    for key, crate in crates.items():
        by_name.setdefault(crate['name'], []).append(key)
    from affected_crates import owning_crate, workspace_crate_dirs
    dirs = workspace_crate_dirs(workspace_packages(metadata))
    seeds, changed_paths, uncertainty = set(), [], []
    changes = request['changes']
    if not isinstance(changes, list) or not changes:
        raise ValueError("at least one proposed change required")
    for change in changes:
        if set(change) == {'package'}:
            value = change['package']
            matches = [value] if value in crates else by_name.get(value, [])
            if len(matches) != 1:
                uncertainty.append('unknown or ambiguous Cargo package: ' + str(value))
                continue
            seeds.add(matches[0])
            changed_paths.append(str(Path(crates[matches[0]]['manifest']).parent).replace('\\', '/'))
        elif set(change) == {'path'}:
            path = relative_path(change['path'])
            (root / path).resolve().relative_to(root)  # reject escaping symlinks
            changed_paths.append(path)
            key = owning_crate(path, dirs, root)
            if key:
                seeds.add(key)
            else:
                uncertainty.append('path has no Cargo owner: ' + path)
        else:
            raise ValueError("change must have exactly package or path")
    # Reuse CI's graph helper over the complete Cargo package inventory too:
    # a non-member adapter can sit between two workspace packages.
    all_packages = {p['id']: p for p in metadata['packages']}
    reverse = reverse_dependencies(metadata, all_packages)
    reachable = set(closure(sorted(seeds), reverse))
    affected = reachable & crates.keys()
    impact_paths = changed_paths + [Path(crates[k]['manifest']).parent.as_posix() for k in affected]
    direct = (set().union(*(reverse[k] for k in seeds)) & crates.keys()) - seeds
    if reachable - crates.keys():
        uncertainty.append('resolved consumers outside workspace require external validation')
    affected_projects, applications, inferred, ownership = set(), set(), set(), []
    cargo_projects = set()
    for key, crate in sorted(crates.items()):
        project = owner(projects, repository, crate['manifest'])
        cargo_projects.add(project)
        if key not in affected:
            continue
        evidence = {'package': key, 'project': project, 'manifest': crate['manifest'],
                    'kind': 'recorded-ownership'}
        ownership.append(evidence)
        affected_projects.add(project)
        if crate['layer'] == 'apps' and project != repository:
            applications.add(project)
        if crate['layer'] == 'apps' and project == repository:
            evidence['inferred_application'] = crate['family']
            inferred.add(crate['family'])
            uncertainty.append('application lacks explicit project ownership: ' + key)
    # Registry projects with no Cargo members may consume shared code through
    # scripts, plugins or deployment. Absence of a Cargo edge is not exclusion.
    non_cargo = set(projects) - cargo_projects - {repository}
    for project in sorted(non_cargo):
        uncertainty.append('project has no Cargo coverage: ' + project)
    inventory = requirements(snapshot)
    applicable = []
    for requirement in inventory:
        scope = requirement['scope']
        if scope['repository'] != repository or not scope['binding']:
            continue
        named = set(scope.get('packages', []))
        unknown_packages = named - crates.keys() - by_name.keys()
        if unknown_packages:
            uncertainty.append('requirement names unknown packages: ' + ', '.join(sorted(unknown_packages)))
        path_match = any(contains(p, c) or contains(c, p) for p in scope.get('paths', []) for c in impact_paths)
        package_match = bool(named & (affected | {crates[k]['name'] for k in affected}))
        scoped_projects = set(scope.get('projects', []))
        unknown = scoped_projects - set(projects)
        if unknown:
            uncertainty.append('requirement names unregistered projects: ' + ', '.join(sorted(unknown)))
            non_cargo.update(unknown)
        # Package/path scopes explicitly record a relationship to the named
        # applications, even when those applications are inactive/non-Cargo.
        if package_match or path_match:
            affected_projects.update(scoped_projects)
            applications.update(scoped_projects)
        global_scope = not any(scope.get(k) for k in ('projects', 'packages', 'paths'))
        if global_scope or package_match or path_match or unknown or unknown_packages or scoped_projects & (affected_projects | non_cargo):
            applicable.append(requirement)
    validation_projects = affected_projects | non_cargo
    broad = any(not u.startswith('project has no Cargo coverage:') for u in uncertainty)
    if broad:
        validation_projects.update(projects)
    # Include project requirements after relationship expansion, independent of
    # inventory order; no semantic rank, token budget or active-app filter here.
    selected = {r['id'] for r in applicable}
    for requirement in inventory:
        scope = requirement['scope']
        project_contract = bool(set(scope.get('projects', [])) & validation_projects)
        non_cargo_contract = any(owner(projects, repository, p) in non_cargo for p in scope.get('paths', []))
        if scope['repository'] == repository and scope['binding'] and (broad or project_contract or non_cargo_contract):
            selected.add(requirement['id'])
    applicable = [r for r in inventory if r['id'] in selected]
    revisions = {r['id']: {'revision': r['revision'], 'fingerprint': r['fingerprint']} for r in inventory}
    old = previous.get('requirement_revisions', {}) if previous else {}
    changed_requirements = sorted(k for k in old.keys() | revisions.keys() if old.get(k) != revisions.get(k)) if previous else []
    packages_to_validate = set(affected)
    if broad:
        packages_to_validate.update(crates)
    else:
        # A requirement can introduce a project that already entered the
        # affected set above. Expand every selected project to its owned Cargo
        # packages rather than using set difference as a proxy for provenance.
        packages_to_validate.update(k for k, c in crates.items() if owner(projects, repository, c['manifest']) in validation_projects)
    return {'schema_version': 1, 'report_kind': 'change-impact', 'source_revision': source_revision,
            'repository': repository, 'changes': changes,
            'input_revisions': {'projects': digest(projects_data), 'requirements': digest(snapshot),
                                'request': digest(request), 'cargo_metadata': cargo_metadata_digest(metadata)},
            'requirement_revisions': revisions, 'changed_requirements': changed_requirements,
            'source_changed': previous is not None and previous.get('source_revision') != source_revision,
            'projects_changed': previous is not None and previous.get('input_revisions', {}).get('projects') != digest(projects_data),
            'changed_packages': sorted(seeds), 'direct_consumers': sorted(direct),
            'transitive_consumers': sorted(affected - seeds - direct),
            'external_consumers': sorted(reachable - crates.keys()),
            'affected_applications': sorted(applications), 'affected_projects': sorted(affected_projects),
            'inferred_applications': sorted(inferred), 'ownership_evidence': ownership,
            'verified_dependencies': sorted([
                {'caller': n['id'], 'dependency': d['pkg'], 'alias': d.get('name'), 'kinds': d.get('dep_kinds', [])}
                for n in metadata['resolve']['nodes'] for d in n['deps']
                if n['id'] in reachable and d['pkg'] in reachable
            ], key=lambda e: (e['caller'], e['dependency'])),
            'requirements': applicable,
            'retrieved_context': snapshot.get('retrieved_context', []),
            'uncertainty': sorted(set(uncertainty)),
            'validation': {'packages': sorted(packages_to_validate), 'projects': sorted(validation_projects),
                           'external_packages': sorted(reachable - crates.keys()),
                           'required_checks': sorted({v for r in applicable for v in r['scope'].get('validation', [])}),
                           'cargo': ['fmt --all -- --check', 'test --frozen --all-features', 'clippy --frozen --all-features --all-targets -- -D warnings'],
                           'status': 'required; not executed', 'scope': 'all features, dev/build and target-specific dependencies'},
            'limitations': ['Snapshot-bound: refresh exports before use; no live service freshness is claimed.',
                            'Retrieved context is inferred, never a binding requirement or verified dependency.',
                            'Requirements remain binding until explicitly withdrawn, including inactive or expired records.']}


def git_revision(repo: str) -> str:
    """Refuse a clean-looking revision for dirty or non-Git source inputs."""
    def git(*args: str) -> str:
        return subprocess.run(['git', '--no-optional-locks', *args], cwd=repo, capture_output=True, text=True,
                              check=True, timeout=10).stdout.strip()
    if git('status', '--porcelain', '--untracked-files=normal'):
        raise ValueError('impact analysis requires a clean checkout; keep snapshots and output outside it')
    return git('rev-parse', 'HEAD')


def impact_main() -> None:
    """Read explicit data files once, run frozen Cargo and recheck source/input revisions."""
    repo, request_file = sys.argv[1:]
    inputs = {}

    def load(path: Path) -> object:
        path = path.resolve()
        data = path.read_bytes()
        inputs[path] = data
        return json.loads(data)

    request_path = Path(request_file)
    request = load(request_path)
    projects = load(request_path.parent / request['projects'])
    snapshot = load(request_path.parent / request['requirements'])
    previous = load(request_path.parent / request['previous']) if request.get('previous') else None
    if previous is not None and (previous.get('schema_version') != 1 or previous.get('report_kind') != 'change-impact'
                                 or previous.get('repository') != request['repository']):
        raise ValueError('previous report must be a supported impact report for this repository')
    revision = git_revision(repo)
    result = subprocess.run(['cargo', 'metadata', '--format-version=1', '--all-features', '--frozen'],
                            cwd=repo, capture_output=True, timeout=90, check=False)
    if result.returncode:
        raise ValueError('cargo metadata failed (requires current lockfile and cached dependencies): '
                         + result.stderr.decode(errors='replace')[:1000])
    metadata = json.loads(result.stdout)
    if os.environ.get('RUSTY_ORCH_METADATA_DIAGNOSTICS') == '1':
        print(json.dumps(cargo_metadata_diagnostics(metadata), sort_keys=True,
                         separators=(',', ':')), file=sys.stderr)
    report = analyze_impact(metadata, request, projects, snapshot, revision, previous)
    if revision != git_revision(repo) or any(path.read_bytes() != data for path, data in inputs.items()):
        raise ValueError('source or snapshot changed during impact analysis; retry')
    print(json.dumps(report, sort_keys=True, indent=2))


if __name__ == '__main__':
    try:
        impact_main() if len(sys.argv) > 2 else inspection_main()
    except (ValueError, KeyError, TypeError, AttributeError, OSError, subprocess.SubprocessError) as error:
        print(f'repository inspection failed: {error}', file=sys.stderr)
        sys.exit(1)

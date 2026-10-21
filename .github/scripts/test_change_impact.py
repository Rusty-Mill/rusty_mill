"""Proposed changes use CI dependency evidence and complete durable inventories."""
import copy
import json
from pathlib import Path
import importlib.util
import sys
import unittest

import test_repository_intelligence as intelligence

sys.modules['repository_intelligence'] = intelligence.MODULE
SPEC = importlib.util.spec_from_file_location('change_impact', Path(intelligence.MODULE.__file__).with_name('impact.py'))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ChangeImpactTests(unittest.TestCase):
    fixture = intelligence.IntelligenceTests.fixture

    def setUp(self):
        self.md = self.fixture([
            ('rusty_atomic_file', 'foundation', 'atomic'), ('orch', 'apps', 'orch'),
            ('remind-core', 'apps', 'remind'), ('remind-cli', 'apps', 'remind'),
            ('board', 'apps', 'board'), ('other', 'apps', 'other'),
            ('spare', 'apps', 'spare')],
            [('orch', 'rusty_atomic_file'), ('remind-core', 'rusty_atomic_file'),
             ('remind-cli', 'remind-core'), ('board', 'rusty_atomic_file')])
        self.projects = {'repos': {'mill': {
            name: [f'crates/{family}'] for name, family in
            [('Orch'.lower(), 'orch'), ('remindme', 'remind'), ('millboard', 'board'),
             ('unrelated', 'other'), ('spare', 'spare')]
        }}}
        self.request = {'schema_version': 1, 'repository': 'mill', 'changes': [{'package': 'rusty_atomic_file'}]}
        self.snapshot = {'schema_version': 1, 'source': 'remind_me', 'complete': True,
                         'records': [], 'retrieved_context': []}

    def requirement(self, key='r', **scope):
        return {'id': key, 'updated_at': 'revision-1', 'content': 'Preserve durable writes',
                'status': 'archived', 'metadata': {'factory_requirement': {
                    'binding': True, 'repository': 'mill', **scope}}}

    def report(self, previous=None):
        return MODULE.analyze_impact(self.md, self.request, self.projects, self.snapshot, 'head-1', previous)

    def test_atomic_identifies_three_apps_with_direct_and_transitive_evidence(self):
        report = self.report()
        self.assertEqual(report['affected_applications'], ['millboard', 'orch', 'remindme'])
        self.assertEqual(report['direct_consumers'], ['board', 'orch', 'remind-core'])
        self.assertEqual(report['transitive_consumers'], ['remind-cli'])
        self.assertEqual(len(report['verified_dependencies']), 4)
        self.assertNotIn('unrelated', report['validation']['projects'])
        self.assertEqual(report['uncertainty'], [])
        self.assertEqual(report['source_revision'], 'head-1')
        self.assertEqual(self.report(), report)

    def test_cargo_metadata_digest_normalizes_schema_defined_sets_only(self):
        package = self.md['packages'][0]
        package.update(
            dependencies=[{'name': 'z', 'features': ['two', 'one'], 'kind': None},
                          {'name': 'a', 'features': []}],
            targets=[{'name': 'z', 'kind': ['lib', 'rlib'], 'crate_types': ['rlib', 'lib'],
                      'required-features': ['two', 'one']},
                     {'name': 'a', 'kind': ['bin'], 'crate_types': ['bin']}],
            features={'default': ['z', 'a']},
            metadata={'application_steps': ['first', 'second']},
        )
        node = self.md['resolve']['nodes'][0]
        node.update(dependencies=['z', 'a'], features=['two', 'one'])
        node['deps'] = [
            {'name': 'z', 'pkg': 'z', 'dep_kinds': [
                {'kind': 'dev', 'target': 'cfg(windows)'}, {'kind': None, 'target': None}]},
            {'name': 'a', 'pkg': 'a', 'dep_kinds': []},
        ]
        equivalent = copy.deepcopy(self.md)
        equivalent['packages'].reverse()
        equivalent['workspace_members'].reverse()
        equivalent['packages'][-1]['dependencies'].reverse()
        equivalent['packages'][-1]['targets'].reverse()
        equivalent['packages'][-1]['features']['default'].reverse()
        for target in equivalent['packages'][-1]['targets']:
            for field in ('kind', 'crate_types', 'required-features'):
                if field in target:
                    target[field].reverse()
        equivalent['resolve']['nodes'].reverse()
        equivalent_node = next(n for n in equivalent['resolve']['nodes'] if n['id'] == node['id'])
        for field in ('dependencies', 'deps', 'features'):
            equivalent_node[field].reverse()
        equivalent_node['deps'][-1]['dep_kinds'].reverse()
        self.assertEqual(MODULE.cargo_metadata_digest(self.md),
                         MODULE.cargo_metadata_digest(equivalent))
        equivalent['packages'][-1]['metadata']['application_steps'].reverse()
        self.assertNotEqual(MODULE.cargo_metadata_digest(self.md),
                            MODULE.cargo_metadata_digest(equivalent))

    def test_cargo_metadata_digest_keeps_meaningful_identity(self):
        package = self.md['packages'][0]
        package.update(dependencies=[{'name': 'dep', 'kind': None, 'features': ['base']}],
                       targets=[{'name': 'library', 'kind': ['lib'], 'crate_types': ['lib']}],
                       features={'default': ['base']})
        baseline = MODULE.cargo_metadata_digest(self.md)
        cases = [
            ('dependency kind', package['dependencies'][0], 'kind', 'build'),
            ('dependency feature', package['dependencies'][0], 'features', ['extra']),
            ('target identity', package['targets'][0], 'name', 'renamed'),
            ('manifest identity', package, 'manifest_path', package['manifest_path'] + '.moved'),
        ]
        for label, subject, field, changed in cases:
            with self.subTest(label=label):
                original = subject[field]
                subject[field] = changed
                self.assertNotEqual(MODULE.cargo_metadata_digest(self.md), baseline)
                subject[field] = original

    def test_cargo_metadata_digest_normalizes_children_before_records(self):
        package = self.md['packages'][0]
        package.update(
            dependencies=[{'name': 'alpha', 'features': ['z', 'a']},
                          {'name': 'beta', 'features': ['m']}],
            targets=[{'name': 'alpha', 'kind': ['z', 'a'], 'crate_types': ['z', 'a'],
                      'required-features': ['z', 'a']},
                     {'name': 'beta', 'kind': ['m'], 'crate_types': ['m'],
                      'required-features': ['m']}],
        )
        node = self.md['resolve']['nodes'][0]
        node['deps'] = [
            {'name': 'alpha', 'pkg': 'alpha', 'dep_kinds': [
                {'kind': 'z', 'target': None}, {'kind': 'a', 'target': None}]},
            {'name': 'beta', 'pkg': 'beta', 'dep_kinds': [
                {'kind': 'm', 'target': None}]},
        ]
        equivalent = copy.deepcopy(self.md)
        equivalent_package = equivalent['packages'][0]
        for dependency in equivalent_package['dependencies']:
            dependency['features'].sort()
        for target in equivalent_package['targets']:
            for field in ('kind', 'crate_types', 'required-features'):
                target[field].sort()
        for dependency in equivalent['resolve']['nodes'][0]['deps']:
            dependency['dep_kinds'].sort(key=lambda kind: kind['kind'])

        self.assertEqual(MODULE.cargo_metadata_digest(self.md),
                         MODULE.cargo_metadata_digest(equivalent))

    def test_cargo_metadata_digest_normalizes_captured_windows_path_spellings(self):
        package = self.md['packages'][0]
        package['manifest_path'] = r'C:\fixture\crate\Cargo.toml'
        package['dependencies'] = [{'name': 'local', 'path': r'C:\fixture\local'}]
        package['targets'] = [{'name': 'crate', 'src_path': r'C:\fixture\crate\src\lib.rs'}]
        self.md.update(workspace_root=r'C:\fixture', target_directory=r'C:\fixture\target',
                       build_directory=r'C:\fixture\target')
        equivalent = copy.deepcopy(self.md)
        extended = lambda value: '\\\\?\\' + value
        equivalent.update(workspace_root=extended(self.md['workspace_root']),
                          target_directory=extended(self.md['target_directory']),
                          build_directory=extended(self.md['build_directory']))
        equivalent_package = equivalent['packages'][0]
        equivalent_package['manifest_path'] = extended(package['manifest_path'])
        equivalent_package['dependencies'][0]['path'] = extended(
            package['dependencies'][0]['path'])
        equivalent_package['targets'][0]['src_path'] = extended(
            package['targets'][0]['src_path'])
        self.assertEqual(MODULE.cargo_metadata_digest(self.md),
                         MODULE.cargo_metadata_digest(equivalent))
        equivalent_package['manifest_path'] += '.moved'
        self.assertNotEqual(MODULE.cargo_metadata_digest(self.md),
                            MODULE.cargo_metadata_digest(equivalent))

    def test_cargo_metadata_digest_preserves_opaque_ids_and_non_windows_paths(self):
        package = self.md['packages'][0]
        baseline = MODULE.cargo_metadata_digest(self.md)
        cases = [
            ('git source id', package, 'id',
             'git+file:///%3F/repo#crate@0.1.0', 'git+file:///repo#crate@0.1.0'),
            ('non-Windows path source id', package, 'id',
             'path+file:///%3F/repo#crate@0.1.0', 'path+file:///repo#crate@0.1.0'),
            ('POSIX manifest path', package, 'manifest_path',
             '/tmp/file:///%3F/crate/Cargo.toml', '/tmp/file:///crate/Cargo.toml'),
        ]
        for label, subject, field, first, second in cases:
            with self.subTest(label=label):
                subject[field] = first
                first_digest = MODULE.cargo_metadata_digest(self.md)
                subject[field] = second
                self.assertNotEqual(MODULE.cargo_metadata_digest(self.md), first_digest)
        package['manifest_path'] = r'\\?\relative\Cargo.toml'
        self.assertNotEqual(MODULE.cargo_metadata_digest(self.md), baseline)

    def test_cargo_metadata_digest_preserves_verbatim_windows_components(self):
        package = self.md['packages'][0]
        package['manifest_path'] = r'C:\fixture\crate.\Cargo.toml'
        package['dependencies'] = [{'name': 'local', 'path': r'C:\fixture\local '}]
        package['targets'] = [
            {'name': 'crate', 'src_path': r'\\server\share\crate.\src\lib.rs'}]
        self.md.update(workspace_root=r'C:\fixture.\workspace',
                       target_directory=r'C:\target.', build_directory=r'C:\build ')

        cases = [
            ('workspace root', self.md, 'workspace_root', r'\\?\C:\fixture.\workspace'),
            ('target directory', self.md, 'target_directory', r'\\?\C:\target.'),
            ('build directory', self.md, 'build_directory', r'\\?\C:\build '),
            ('manifest path', package, 'manifest_path', r'\\?\C:\fixture\crate.\Cargo.toml'),
            ('dependency path', package['dependencies'][0], 'path', r'\\?\C:\fixture\local '),
            ('target source path', package['targets'][0], 'src_path',
             r'\\?\UNC\server\share\crate.\src\lib.rs'),
        ]
        for label, subject, field, extended in cases:
            with self.subTest(label=label):
                ordinary = subject[field]
                ordinary_digest = MODULE.cargo_metadata_digest(self.md)
                subject[field] = extended
                self.assertNotEqual(MODULE.cargo_metadata_digest(self.md), ordinary_digest)
                subject[field] = ordinary

    def test_metadata_diagnostics_are_redacted_bounded_and_canonical(self):
        self.md['metadata'] = {'DO_NOT_PRINT_SENTINEL': {'private': 'DO_NOT_PRINT_VALUE'}}
        self.md['packages'][0]['metadata'] = {'ANOTHER_SENTINEL': ['secret']}
        equivalent = copy.deepcopy(self.md)
        equivalent['packages'].reverse()
        diagnostics = MODULE.cargo_metadata_diagnostics(self.md)
        self.assertEqual(diagnostics, MODULE.cargo_metadata_diagnostics(equivalent))
        encoded = json.dumps(diagnostics, sort_keys=True, separators=(',', ':'))
        self.assertLessEqual(len(encoded.encode()), 8192)
        self.assertNotIn('DO_NOT_PRINT_SENTINEL', encoded)
        self.assertNotIn('DO_NOT_PRINT_VALUE', encoded)
        self.assertNotIn('ANOTHER_SENTINEL', encoded)

    def test_metadata_diagnostics_classify_paths_and_empty_containers(self):
        package = self.md['packages'][0]
        package['dependencies'] = [{'name': 'local', 'path': r'\\?\C:\checkout\local',
                                    'features': []}]
        package['metadata'] = []
        diagnostics = MODULE.cargo_metadata_diagnostics(self.md)
        by_pointer = {entry['pointer']: entry for entry in diagnostics['records']}
        path = next(entry for pointer, entry in by_pointer.items()
                    if pointer.endswith('/dependencies/0/path'))
        self.assertEqual(path['path_spelling'], 'ordinary')
        self.assertEqual(diagnostics['schema_version'], 2)
        self.assertEqual(diagnostics['path_fields']['dependency.path']['spellings']['ordinary'], 1)
        self.assertTrue(any(entry['type'] == 'array' and entry.get('opaque') == 'authored-metadata'
                            for entry in diagnostics['records']))
        empty_object = MODULE.cargo_metadata_diagnostics({'resolve': {}})['records'][0]
        empty_array = MODULE.cargo_metadata_diagnostics({'packages': []})['records'][0]
        self.assertEqual((empty_object['type'], empty_array['type']), ('object', 'array'))
        self.assertNotEqual(empty_object['fingerprint'], empty_array['fingerprint'])

    def test_metadata_diagnostics_enforce_record_byte_and_depth_limits(self):
        metadata = {'packages': [{'id': f'package-{index}', 'name': 'x' * 100}
                                 for index in range(1000)]}
        diagnostics = MODULE.cargo_metadata_diagnostics(metadata)
        encoded = json.dumps(diagnostics, sort_keys=True, separators=(',', ':')).encode()
        self.assertLessEqual(len(encoded), 8192)
        self.assertTrue(diagnostics['truncated']['records'] or diagnostics['truncated']['bytes'])
        nested = value = {}
        for index in range(20):
            value['child'] = {}
            value = value['child']
        self.assertGreater(MODULE.cargo_metadata_diagnostics(nested)['truncated']['depth'], 0)

    def test_metadata_diagnostics_summarize_changes_beyond_detail_limit(self):
        metadata = {'packages': [{'id': f'package-{index}', 'name': 'x' * 100}
                                 for index in range(100)]}
        before = MODULE.cargo_metadata_diagnostics(metadata)
        metadata['packages'][-1]['name'] = 'changed beyond detail prefix'
        after = MODULE.cargo_metadata_diagnostics(metadata)
        self.assertNotEqual(before['top_level_fingerprints'], after['top_level_fingerprints'])
        self.assertNotEqual(before['package_remainder'], after['package_remainder'])

    def test_rank_never_omits_binding_inactive_requirements(self):
        self.projects['repos']['mill']['dormant'] = ['scripts/dormant']
        self.snapshot['records'] = [self.requirement(projects=['dormant'], packages=['rusty_atomic_file'],
                                                     validation=['dormant compatibility suite'])]
        report = self.report()
        self.assertIn('dormant', report['affected_applications'])
        self.assertEqual([r['id'] for r in report['requirements']], ['r'])
        self.assertEqual(report['requirements'][0]['lifecycle']['status'], 'archived')
        self.assertEqual(report['validation']['required_checks'], ['dormant compatibility suite'])
        self.assertEqual(report['retrieved_context'], [])

    def test_requirement_linked_cargo_project_expands_validation_packages(self):
        self.snapshot['records'] = [self.requirement(projects=['unrelated'],
                                                     packages=['rusty_atomic_file'])]
        report = self.report()
        self.assertIn('unrelated', report['affected_applications'])
        self.assertIn('other', report['validation']['packages'])
        self.assertNotIn('spare', report['validation']['packages'])
        self.assertNotIn('spare', report['validation']['projects'])

    def test_revision_scope_content_withdrawal_and_removal_changes_detected(self):
        self.snapshot['records'] = [self.requirement(projects=['orch'])]
        before = self.report()
        for field, value in [('updated_at', 'revision-2'), ('content', 'New contract')]:
            with self.subTest(field=field):
                original = self.snapshot['records'][0][field]
                self.snapshot['records'][0][field] = value
                self.assertEqual(self.report(before)['changed_requirements'], ['r'])
                self.snapshot['records'][0][field] = original
        self.snapshot['records'][0]['metadata']['factory_requirement']['binding'] = False
        withdrawn = self.report(before)
        self.assertEqual(withdrawn['requirements'], [])
        self.assertEqual(withdrawn['changed_requirements'], ['r'])
        self.snapshot['records'] = []
        self.assertEqual(self.report(before)['changed_requirements'], ['r'])

    def test_project_requirements_selected_after_relationship_expansion(self):
        self.projects['repos']['mill']['dormant'] = ['scripts/dormant']
        self.snapshot['records'] = [self.requirement('a', projects=['dormant']),
                                   self.requirement('z', projects=['dormant'], packages=['rusty_atomic_file'])]
        self.assertEqual([r['id'] for r in self.report()['requirements']], ['a', 'z'])

    def test_unknown_paths_packages_and_non_cargo_consumers_broaden_scope(self):
        self.projects['repos']['mill']['plugin'] = ['plugins/ui']
        self.snapshot['records'] = [self.requirement('plugin-path', paths=['plugins/ui']),
                                   self.requirement('other-package', packages=['other'])]
        self.assertIn('plugin', self.report()['validation']['projects'])
        self.assertEqual([r['id'] for r in self.report()['requirements']], ['plugin-path'])
        for change in [{'path': 'Cargo.toml'}, {'package': 'missing'}]:
            self.request['changes'] = [change]
            report = self.report()
            self.assertIn('unrelated', report['validation']['projects'])
            self.assertIn('other', report['validation']['packages'])
            self.assertTrue(report['uncertainty'])
            self.assertEqual([r['id'] for r in report['requirements']], ['other-package', 'plugin-path'])

    def test_deleted_path_and_renamed_optional_target_build_edges(self):
        self.request['changes'] = [{'path': 'crates/atomic/crates/rusty_atomic_file/deleted.rs'}]
        edge = self.md['resolve']['nodes'][1]['deps'][0]
        edge['name'] = 'renamed_atomic'
        edge['dep_kinds'] = [{'kind': 'build', 'target': 'cfg(windows)'}]
        report = self.report()
        self.assertEqual(report['changed_packages'], ['rusty_atomic_file'])
        evidence = next(e for e in report['verified_dependencies'] if e['caller'] == 'orch')
        self.assertEqual(evidence['alias'], 'renamed_atomic')
        self.assertEqual(evidence['kinds'][0]['target'], 'cfg(windows)')

    def test_incomplete_inventory_malformed_requirement_and_traversal_fail(self):
        self.snapshot['complete'] = False
        with self.assertRaises(ValueError):
            self.report()
        self.snapshot['complete'] = True
        self.snapshot['records'] = [self.requirement()]
        del self.snapshot['records'][0]['updated_at']
        with self.assertRaises(ValueError):
            self.report()
        self.snapshot['records'] = []
        for path in ['../outside', '/outside', 'C:/outside', 'a/../b', 'a\\b']:
            self.request['changes'] = [{'path': path}]
            with self.assertRaises(ValueError):
                self.report()

    def test_unknown_requirement_scope_is_not_silently_dropped(self):
        self.snapshot['records'] = [self.requirement(packages=['external-sdk'])]
        report = self.report()
        self.assertEqual(len(report['requirements']), 1)
        self.assertIn('unrelated', report['validation']['projects'])

    def test_consumer_path_contract_and_unregistered_inactive_project_are_included(self):
        self.snapshot['records'] = [self.requirement(paths=['crates/remind/crates/remind-core/src']),
                                   self.requirement('inactive', projects=['unregistered'])]
        report = self.report()
        self.assertEqual([r['id'] for r in report['requirements']], ['inactive', 'r'])
        self.assertIn('unregistered', report['validation']['projects'])

    def test_retrieval_access_signals_do_not_change_requirement_revisions(self):
        self.snapshot['records'] = [self.requirement(projects=['orch'])]
        before = self.report()
        self.snapshot['records'][0].update(access_count=42, vitality=0.5, accessed_at='later')
        self.assertEqual(self.report(before)['changed_requirements'], [])

    def test_null_requirement_marker_and_duplicate_ids_fail_closed(self):
        self.snapshot['records'] = [{'metadata': {'factory_requirement': None}}]
        with self.assertRaises(ValueError):
            self.report()
        self.snapshot['records'] = [self.requirement(), self.requirement()]
        with self.assertRaises(ValueError):
            self.report()

    def test_missing_application_ownership_is_inferred_and_broadens_validation(self):
        del self.projects['repos']['mill']['orch']
        report = self.report()
        self.assertEqual(report['inferred_applications'], ['orch'])
        self.assertNotIn('orch', report['affected_applications'])
        self.assertIn('unrelated', report['validation']['projects'])

    def test_millboard_name_order_and_component_boundaries(self):
        projects = MODULE.registry({'repos': {'mill': {'a': ['app'], 'z': ['app/sub']},
                                              'another': {'remote': ['app']}}}, 'mill')
        self.assertEqual(MODULE.owner(projects, 'mill', 'app/sub/file'), 'a')
        self.assertEqual(MODULE.owner(projects, 'mill', 'apple/file'), 'mill')
        self.assertNotIn('remote', projects)

    def test_ambiguous_names_are_not_treated_as_package_identities(self):
        self.md['packages'][-1]['name'] = 'rusty_atomic_file'
        report = self.report()  # exact id continues to identify original package
        self.assertEqual(report['changed_packages'], ['rusty_atomic_file'])
        self.md['packages'][0]['id'] = 'original-id'
        self.md['workspace_members'][0] = 'original-id'
        self.md['resolve']['nodes'][0]['id'] = 'original-id'
        for node in self.md['resolve']['nodes']:
            for dep in node['deps']:
                if dep['pkg'] == 'rusty_atomic_file':
                    dep['pkg'] = 'original-id'
        self.assertTrue(self.report()['uncertainty'])

    def test_external_adapter_edges_preserve_evidence(self):
        atomic = self.md['resolve']['nodes'][1]['deps'][0]
        atomic['pkg'] = 'adapter'
        self.md['packages'].append({'id': 'adapter', 'name': 'adapter', 'source': 'path+external'})
        self.md['resolve']['nodes'].append({'id': 'adapter', 'deps': [{'name': 'atomic', 'pkg': 'rusty_atomic_file'}]})
        report = self.report()
        self.assertIn('orch', report['transitive_consumers'])
        self.assertTrue(any(e['caller'] == 'adapter' for e in report['verified_dependencies']))


if __name__ == '__main__':
    unittest.main()

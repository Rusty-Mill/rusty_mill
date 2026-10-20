"""Proposed changes use CI dependency evidence and complete durable inventories."""
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
            ('board', 'apps', 'board'), ('other', 'apps', 'other')],
            [('orch', 'rusty_atomic_file'), ('remind-core', 'rusty_atomic_file'),
             ('remind-cli', 'remind-core'), ('board', 'rusty_atomic_file')])
        self.projects = {'repos': {'mill': {
            name: [f'crates/{family}'] for name, family in
            [('Orch'.lower(), 'orch'), ('remindme', 'remind'), ('millboard', 'board'), ('unrelated', 'other')]
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

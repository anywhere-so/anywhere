"""Protect route parameters, transitive refs, and fail-closed schema generation."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('client_schema', Path(__file__).with_name('client-schema.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ProjectionTests(unittest.TestCase):
    def schema(self):
        return {
            'paths': {
                '/cli': {name: {'operationId': name} for name in module.OPERATIONS},
                '/example/{id}': {
                    'parameters': [{'$ref': '#/components/parameters/Id'}],
                    'get': {'operationId': 'app_view', 'responses': {'200': {'$ref': '#/components/responses/Ok'}}},
                    'post': {'operationId': 'unrelated'},
                },
                '/unrelated': {'get': {'operationId': 'unrelated'}},
            },
            'components': {
                'parameters': {'Id': {'name': 'id', 'in': 'path', 'required': True, 'schema': {'type': 'string'}}},
                'responses': {'Ok': {'schema': {'$ref': '#/components/schemas/App'}}},
                'schemas': {'App': {'properties': {'next': {'$ref': '#/components/schemas/App'}}}, 'Unused': {}},
            },
        }

    def test_preserves_path_parameters_and_recursive_references(self):
        result = module.project(self.schema())
        self.assertEqual(result['paths']['/example/{id}']['parameters'], [{'$ref': '#/components/parameters/Id'}])
        self.assertEqual(result['components']['schemas']['App'], self.schema()['components']['schemas']['App'])
        self.assertNotIn('Unused', result['components']['schemas'])
        self.assertNotIn('post', result['paths']['/example/{id}'])
        self.assertNotIn('/unrelated', result['paths'])

    def test_missing_cli_operation_fails(self):
        schema = self.schema()
        del schema['paths']['/cli']['app_delete']
        with self.assertRaisesRegex(ValueError, 'app_delete'):
            module.project(schema)

    def test_external_reference_fails(self):
        schema = self.schema()
        schema['components']['responses']['Ok'] = {'$ref': 'https://other.test/schema.json'}
        with self.assertRaisesRegex(ValueError, 'Unsupported schema reference'):
            module.project(schema)


if __name__ == '__main__':
    unittest.main()

#!/usr/bin/env python3
"""Project a service OpenAPI schema onto the Anywhere CLI's HTTP surface.

Usage: python3 tools/client-schema.py SERVICE_SCHEMA OUTPUT
Wire definitions are preserved; unrelated operations and unreachable schemas are omitted.
"""
import json
from pathlib import Path
import sys

OPERATIONS = {
    'account_usage',
    'workspace_list',
    'workspace_usage',
    'workspace_create',
    'workspace_view',
    'workspace_members',
    'workspace_invite',
    'workspace_invitation_revoke',
    'workspace_invitation_accept',
    'workspace_member_update',
    'workspace_member_remove',
    'workspace_apps',
    'workspace_app_sharing',
    'workspace_app_sharing_set',
    'workspace_app_grant',
    'workspace_app_ungrant',

    'app_access_get',
    'app_access_set',
    'app_delete',
    'app_list',
    'app_logs',
    'app_ticket_create',
    'app_usage',
    'app_view',
    'cli_login_create',
    'cli_login_token',
    'deploy_create',
    'me_view',
    'operation_cancel',
    'operation_view',
    'secret_list',
    'secret_set',
    'secret_unset',
    'secrets_set',
    'share_add',
    'share_list',
    'share_remove',
    'shared_app_list',
}


def project(schema):
    schema = json.loads(json.dumps(schema))
    found = set()
    paths = {}
    for path, item in schema['paths'].items():
        kept = {}
        for method, operation in item.items():
            if isinstance(operation, dict) and operation.get('operationId') in OPERATIONS:
                kept[method] = operation
                found.add(operation['operationId'])
        if kept:
            for key in ('parameters', 'summary', 'description', 'servers'):
                if key in item:
                    kept[key] = item[key]
            paths[path] = kept
    missing = OPERATIONS - found
    if missing:
        raise ValueError(f'CLI operations missing from source schema: {sorted(missing)}')
    schema['paths'] = paths
    components = schema.get('components', {})
    retained = {}

    def visit(value):
        if isinstance(value, dict):
            ref = value.get('$ref')
            if ref:
                if not ref.startswith('#/components/'):
                    raise ValueError(f'Unsupported schema reference: {ref}')
                category, name = ref[len('#/components/'):].split('/')
                target = retained.setdefault(category, {})
                if name not in target:
                    target[name] = components[category][name]
                    visit(target[name])
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    visit({key: value for key, value in schema.items() if key != 'components'})
    # Security requirements name schemes without a JSON $ref.
    if 'securitySchemes' in components:
        retained['securitySchemes'] = components['securitySchemes']
    schema['components'] = retained
    return schema


if __name__ == '__main__':
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    source, destination = map(Path, sys.argv[1:])
    result = project(json.loads(source.read_text()))
    destination.write_text(json.dumps(result, indent=2) + '\n')

# Teams and project destinations

A person keeps a free personal workspace and may join teams for free. Personal
Pro is optional. Team app use needs membership and app access; team development
also needs an assigned team-paid developer seat. A seat never exposes other
restricted apps. Workspace owners manage members and billing. An app's creator
and workspace owners manage that app; management alone grants no runtime access.

```sh
anywhere workspaces list
anywhere --workspace acme apps list
anywhere deploy . --workspace acme
anywhere exec acme/wiki -- ls
anywhere logs --app acme/wiki
anywhere pull acme/wiki /app ./wiki
anywhere --workspace acme usage
```

An unbound deploy with more than one workspace requires an explicit choice.
`WORKSPACE/APP` and `--workspace WORKSPACE` must agree. A successful folder deploy
saves `.anywhere/project.json`: API origin, workspace UUID and app UUID. Later
folder commands use it. This directory is never uploaded. A missing app, reused
name, wrong API origin, revoked membership or conflicting selector stops the
command. Never retry by changing the destination to personal or recreating it.

```sh
anywhere workspaces bind acme/wiki ./wiki
anywhere workspaces bind other/wiki ./wiki --replace
anywhere workspaces unbind ./wiki
```

Binding changes local destination metadata only; it never transfers an app or its
charges. `--replace` is an explicit choice after inspecting the destination. Keep
separate folders for agents targeting different workspaces. There is no global
active workspace.

Owners invite people by their verified sign-in email and send the resulting link:

```sh
anywhere workspaces create acme --title 'Acme'
anywhere workspaces invite acme ada@example.com
anywhere workspaces members acme
anywhere workspaces set acme USER_UUID --role member --developer true
anywhere workspaces remove acme USER_UUID
```

Invitations do not buy seats. Assign only already purchased capacity. Developer
access permits deploy, exec, pull, logs and secrets, and removing it cannot undo
changes already made. The last owner cannot leave; appoint another owner first.
Removed members lose their team access through every credential, while their
personal access remains.

App managers set direct grants and workspace-wide use:

```sh
anywhere workspaces sharing acme wiki
anywhere workspaces grant acme wiki USER_UUID --role member
anywhere workspaces grant acme wiki USER_UUID --role developer
anywhere workspaces revoke-grant acme wiki USER_UUID
anywhere workspaces visibility acme wiki --everyone true
anywhere workspaces visibility acme wiki --everyone false
```

`everyone` means current workspace members, never the public internet. Restricted
is the default. Direct grants and everyone combine; removing one source does not
remove another. Team groups come later. Personal `anywhere shares` stays free and
separate. A team developer pays nothing for app use; a workspace owner explicitly
buys the team's seats, $10 per developer monthly, and top-ups in the portal's
Usage & billing. One $10 runtime allowance belongs to the workspace regardless of
seat count. No personal Pro subscription is needed. Billing links are browser-only.

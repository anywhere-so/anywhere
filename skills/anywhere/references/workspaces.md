# Teams and project destinations

A person keeps a free personal workspace and may join teams for free. Personal
Pro is optional. A new team app is visible to everyone in its workspace: every
member uses it, and every member with an assigned team-paid vibecoder seat
develops it. Without a seat, nobody develops. A restricted app is reached only
by workspace owners, its creator and members granted it; a seat never exposes
it. Workspace owners manage members and billing. An app's creator and workspace
owners manage that app.

```sh
anywhere workspaces list
anywhere --workspace acme apps list
anywhere deploy ~/.anywhere/apps/acme/wiki --workspace acme
anywhere exec acme/wiki -- ls
anywhere logs --app acme/wiki
anywhere pull acme/wiki /app ~/.anywhere/apps/acme/wiki
anywhere --workspace acme usage
```

An unbound deploy with more than one workspace requires an explicit choice.
`WORKSPACE/APP` and `--workspace WORKSPACE` must agree. A successful folder deploy
saves `.anywhere/project.json`: API origin, workspace UUID and app UUID. Later
folder commands use it. This directory is never uploaded. A missing app, reused
name, wrong API origin, revoked membership or conflicting selector stops the
command. Never retry by changing the destination to personal or recreating it.

```sh
anywhere workspaces bind acme/wiki ~/.anywhere/apps/acme/wiki
anywhere workspaces bind other/wiki ~/.anywhere/apps/other/wiki --replace
anywhere workspaces unbind ~/.anywhere/apps/acme/wiki
```

Binding changes local destination metadata only; it never transfers an app or its
charges. `--replace` is an explicit choice after inspecting the destination. Keep
separate folders for agents targeting different workspaces. There is no global
active workspace.

A team exists only once a person pays for it, so an agent can't create one.
`create` checks the name, holding nothing, and prints `checkout_url`: give that
link to your person, who pays $50 a month on Stripe in their signed-in browser.
Once paid, the team exists, and its creator owns it on one of its 5 included
vibecoder seats, so they can deploy at once. A name that's taken or held by
someone else's checkout is refused with `username_taken` or
`workspace_name_held`; choose another.

```sh
anywhere workspaces create acme --title 'Acme'
```

Owners invite people by their verified sign-in email and send the resulting link:

```sh
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

App managers see each member's access and why, set per-app grants and choose
who sees the app:

```sh
anywhere workspaces sharing acme wiki
anywhere workspaces grant acme wiki USER_UUID --role member
anywhere workspaces grant acme wiki USER_UUID --role developer
anywhere workspaces revoke-grant acme wiki USER_UUID
anywhere workspaces visibility acme wiki --everyone true
anywhere workspaces visibility acme wiki --everyone false
```

`everyone` means current workspace members, never the public internet, and is
the default. On such an app a grant overrides the default for one member:
`member` turns a seated member down to use only, and `developer` is the same as
the default. On a restricted app, a grant is the only way in. Owners and the
app's creator can't be turned down, and nobody can be blocked from an app
visible to everyone: restrict it instead. Team groups come later. Personal `anywhere shares` stays free and
separate. A team member pays nothing for app use. A team is $50 a month with 5
vibecoder seats and one shared $50 runtime allowance, whatever its seat count;
a workspace owner explicitly buys more seats, $10 each monthly, and top-ups in
the portal's Usage & billing. No personal Pro subscription is needed. Billing
links are browser-only.

An unbound folder cannot choose a destination for logs or secrets when you belong to teams. Pass `--workspace NAMESPACE` or `--app WORKSPACE/APP`, or explicitly bind the project. Local input validation still happens before any request. A folder copied with `pull` is unbound until deployed or explicitly bound.

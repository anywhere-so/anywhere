use super::*;

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

/// What the control API answers for notes, a small app on Free with a 10 GiB
/// disk, over the last 24 hours, running and healthy: as `cli.md`'s first
/// example.
fn healthy() -> Value {
    json!({
        "cost": null, "app": "notes", "status": "running", "restarts": 0,
        "range": "24h", "from": "2026-10-26T14:30:00Z", "to": "2026-10-27T14:30:00Z",
        "step_seconds": 300, "problems_from": "2026-10-26T14:30:00Z",
        "limits": { "cpu_millicores": 1000, "memory_bytes": 2 * GIB, "disk_bytes": 10 * GIB },
        "latest": { "at": "2026-10-27T14:29:41Z", "cpu_millicores": 320, "memory_bytes": 412 * MIB },
        "disk_used_bytes": 2_340_757_504u64,
        "totals": {
            "seconds_with_readings": 86400, "cpu_avg_millicores": 210, "cpu_peak_millicores": 930,
            "cpu_throttled_seconds": 0.0, "memory_avg_bytes": 398 * MIB,
            "memory_peak_bytes": 486 * MIB, "oom_kills": 0,
            "bytes_in": 1_702_334_112u64, "bytes_out": 6_593_120_040u64
        },
        "gaps": [],
        "problems": [],
        "points": [{
            "cost_microusd": null,
            "at": "2026-10-27T14:25:00Z", "seconds": 300, "cpu_avg_millicores": 320,
            "cpu_peak_millicores": 410, "cpu_throttled_seconds": 0.0,
            "memory_avg_bytes": 412 * MIB, "memory_peak_bytes": 420 * MIB,
            "disk_used_bytes": 2_340_757_504u64, "bytes_in": 5_000_000, "bytes_out": 20_000_000,
            "oom_kills": 0
        }]
    })
}

/// notes again, after a deploy that leaks: it ran out of memory 6 times.
fn out_of_memory() -> Value {
    let mut usage = healthy();
    usage["restarts"] = json!(19);
    usage["latest"]["memory_bytes"] = json!(1_267_015_680u64);
    usage["totals"]["memory_avg_bytes"] = json!(1_095_216_660u64);
    usage["totals"]["memory_peak_bytes"] = json!(2 * GIB);
    usage["totals"]["oom_kills"] = json!(6);
    usage["problems"] = json!([{
        "code": "oom_kills", "severity": "failure",
        "message": "It ran out of its 2 GiB of memory 6 times in the last 24 hours; each time every process was ended and the app started again. Its memory is what its sandbox holds on the host, gVisor's own included but not the files it has cached.",
        "fix": "This app's memory ceiling is 2 GiB. Pro allows up to 16 GiB per app; upgrade at https://anywhere.so/billing. Or lower what the app uses, such as Node's --max-old-space-size or fewer workers.",
        "change": { "file": "app.toml", "table": "limits", "key": "memory", "value": "4 GiB" },
        "needs_plan": "pro", "upgrade_url": "https://anywhere.so/billing",
        "last_at": "2026-10-27T14:05:00Z",
        "agent_prompt": "My anywhere app \"Notes\" (notes) ran into its limits over the last 24 hours."
    }]);
    usage
}

#[tokio::test]
async fn shows_a_healthy_apps_usage_against_its_limits_in_one_layout() {
    let server = Server::start(vec![reply(healthy())]).await;
    let output = server.run(&["usage", "notes"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "\
notes: up to 1 shared vCPU, 2 GiB memory, 10 GiB disk; running
last 24 hours, a point every 5 minutes; readings for all of it

          now         average     peak        limit
cpu       0.32 vCPU   0.21 vCPU   0.93 vCPU   1 shared vCPU     at its limit 0% of the time
memory    412 MiB     398 MiB     486 MiB     2 GiB             never ran out
disk      2.18 GiB                            10 GiB            21% used, 7.82 GiB free
network   in 1.7 GB, out 6.6 GB over the range

No problems in the last 24 hours.
"
    );
    assert_eq!(text(&output.stderr), "");
    assert_eq!(
        server.paths(),
        ["GET /v1/apps/notes/usage?range=24h HTTP/1.1"]
    );
}

#[tokio::test]
async fn after_a_deploy_the_problems_count_from_it_and_say_so() {
    let mut usage = healthy();
    usage["problems_from"] = json!("2026-10-27T13:00:00Z");
    let server = Server::start(vec![reply(usage)]).await;
    let output = server.run(&["usage", "notes"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).ends_with("\nNo problems since its last deploy.\n"),
        "{}",
        text(&output.stdout)
    );
}

#[tokio::test]
async fn a_problem_comes_with_its_fix_and_the_command_still_succeeds() {
    let server = Server::start(vec![reply(out_of_memory())]).await;
    let output = server.run(&["usage", "notes"]).await;
    // A problem is part of the answer, not a failure of the command.
    assert!(output.status.success(), "{}", text(&output.stderr));
    let shown = text(&output.stdout);
    assert!(
        shown.starts_with(
            "notes: up to 1 shared vCPU, 2 GiB memory, 10 GiB disk; running, restarted 19 times\n"
        ),
        "{shown}"
    );
    assert!(
        shown.contains(
            "memory    1.18 GiB    1.02 GiB    2 GiB       2 GiB             ran out 6 times\n"
        ),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            "\nproblem: oom_kills: it ran out of its 2 GiB of memory 6 times in the last 24 hours; each time every process was ended and the app started again. Its memory is what its sandbox holds on the host, gVisor's own included but not the files it has cached. The last was counted in the 5 minutes from 14:05 UTC.
fix: this app's memory ceiling is 2 GiB. Pro allows up to 16 GiB per app; upgrade at https://anywhere.so/billing. Or lower what the app uses, such as Node's --max-old-space-size or fewer workers.
"
        ),
        "{shown}"
    );
    assert!(!shown.contains("No problems"));
}

#[tokio::test]
async fn json_includes_points_only_with_detail_and_omits_paste_prompts() {
    let mut usage = out_of_memory();
    usage["points"][0]["cost_microusd"] = json!(4321);
    let server = Server::start(vec![reply(usage.clone())]).await;
    usage["problems"][0]
        .as_object_mut()
        .unwrap()
        .remove("agent_prompt");
    let output = server
        .run(&[
            "--json",
            "usage",
            "michael/notes",
            "--range",
            "7d",
            "--detail",
        ])
        .await;
    assert_eq!(success(&output), usage);
    assert_eq!(text(&output.stderr), "");
    assert_eq!(
        server.paths(),
        ["GET /v1/apps/notes/usage?owner=michael&range=7d HTTP/1.1"]
    );
    usage.as_object_mut().unwrap().remove("points");
    let output = server.run(&["--json", "usage", "michael/notes"]).await;
    assert_eq!(success(&output), usage);
}

#[tokio::test]
async fn a_range_without_an_app_is_refused_before_any_request() {
    let server = Server::start(vec![reply(account())]).await;
    // Even the normal app default must be refused when explicitly given.
    for range in ["1h", "24h", "7d", "30d"] {
        let output = server.run(&["--json", "usage", "--range", range]).await;
        let error = failure(&output, "argument_invalid");
        assert_eq!(error["retryable"], false);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("anywhere usage NAME --range")
        );
    }
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn a_crashed_app_says_when_it_stopped_and_points_to_its_logs() {
    let mut usage = healthy();
    usage["status"] = json!("crashed");
    usage["restarts"] = json!(10);
    usage["latest"] = Value::Null;
    usage["totals"]["seconds_with_readings"] = json!(68_580);
    usage["gaps"] = json!([
        { "from": "2026-10-26T22:10:00Z", "to": "2026-10-27T00:20:00Z" },
        { "from": "2026-10-27T02:00:00Z", "to": "2026-10-27T06:30:00Z" },
        { "from": "2026-10-27T14:02:00Z", "to": null }
    ]);
    let server = Server::start(vec![reply(usage)]).await;
    let output = server.run(&["usage", "notes"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let shown = text(&output.stdout);
    assert!(
        shown.contains("; crashed, restarted 10 times\nlast 24 hours, a point every 5 minutes; readings for 19 h 3 min of it\n"),
        "{shown}"
    );
    assert!(
        shown.contains("\ncpu       -           0.21 vCPU"),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            "\nno readings: 26 Oct 22:10 to 27 Oct 00:20 UTC\nno readings: 02:00 to 06:30 UTC\nno readings: since 14:02 UTC\n"
        ),
        "{shown}"
    );
    // On standard error, as `anywhere pull` prints them, so the command pastes.
    assert_eq!(
        text(&output.stderr),
        "next: anywhere logs --app notes\nnote: the logs say why it crashed; its next deploy or change to its secrets starts it again.\n"
    );
}

#[tokio::test]
async fn an_app_with_no_readings_yet_says_when_the_first_comes() {
    let mut usage = healthy();
    usage["latest"] = Value::Null;
    usage["disk_used_bytes"] = Value::Null;
    usage["points"] = json!([]);
    usage["gaps"] = json!([{ "from": "2026-10-26T14:30:00Z", "to": null }]);
    usage["totals"] = json!({
        "seconds_with_readings": 0, "cpu_avg_millicores": null, "cpu_peak_millicores": null,
        "cpu_throttled_seconds": 0.0, "memory_avg_bytes": null, "memory_peak_bytes": null,
        "oom_kills": 0, "bytes_in": 0, "bytes_out": 0
    });
    let server = Server::start(vec![reply(usage)]).await;
    let output = server.run(&["usage", "notes"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "\
notes: up to 1 shared vCPU, 2 GiB memory, 10 GiB disk; running
last 24 hours; no readings yet

          now         average     peak        limit
cpu       -           -           -           1 shared vCPU
memory    -           -           -           2 GiB
disk      -                                   10 GiB
network   none yet

No readings yet: its host reads what it uses every 30 seconds while it runs.
"
    );
}

#[tokio::test]
async fn a_range_it_doesnt_know_is_refused_before_any_request() {
    let server = Server::start(vec![reply(healthy())]).await;
    let output = server
        .run(&["--json", "usage", "notes", "--range", "90d"])
        .await;
    assert_eq!(output.status.code(), Some(2));
    let error = failure(&output, "argument_invalid");
    assert_eq!(error["retryable"], false);
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn the_platforms_refusal_keeps_its_code_and_says_whether_to_retry() {
    let refusal = json!({"code": "member_only", "retryable": false,
        "message": "michael made you a member of wiki, to use it but not change it; ask them to make you a developer"});
    let server = Server::start(vec![Reply {
        status: 403,
        ..reply(refusal)
    }])
    .await;
    let output = server.run(&["--json", "usage", "michael/wiki"]).await;
    assert_eq!(output.status.code(), Some(1));
    let error = failure(&output, "member_only");
    assert_eq!(error["retryable"], false);
    assert_eq!(server.paths().len(), 1);
}

fn account() -> Value {
    json!({
        "plan": "free", "period": {"from": "2026-10-01T00:00:00Z", "resets_at": "2026-11-01T00:00:00Z"},
        "charged_through": "2026-10-28T19:00:00Z", "included_microusd": 3000000, "available_microusd": 3000000, "topup_balance_microusd": 0, "topup_amounts_microusd": [], "state": "ok",
        "used_microusd": 1230000, "percent_used": 41, "forecast_microusd": 1650000,
        "forecast_percent": 55, "enforced": true,
        "apps": [{"app": "blog", "title": "Blog", "deleted": false, "used_microusd": 620000,
            "forecast_microusd": 900000, "memory_microusd": 400000, "cpu_microusd": 90000,
            "disk_microusd": 80000, "egress_microusd": 50000}],
        "rates": {"memory_gib_hour_microusd": 25000, "cpu_vcpu_hour_microusd": 100000,
            "disk_gib_month_microusd": 300000, "egress_gib_microusd": 200000}
    })
}

#[tokio::test]
async fn before_any_charge_it_says_so_and_dollars_round_down() {
    let mut fresh = account();
    fresh["charged_through"] = Value::Null;
    fresh["used_microusd"] = json!(2_999_999);
    fresh["apps"] = json!([]);
    let server = Server::start(vec![reply(fresh)]).await;
    let output = server.run(&["usage"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).starts_with(
            "Usage this month (Free): 41%, $2.99 of $3.00 · nothing charged yet: each hour is charged after it ends · resets 1 Nov\n"
        ),
        "{}",
        text(&output.stdout)
    );
}

#[tokio::test]
async fn no_app_reads_the_account_and_prints_the_contract_layout() {
    let server = Server::start(vec![reply(account())]).await;
    let output = server.run(&["usage"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "\
Usage this month (Free): 41%, $1.23 of $3.00 · on track for about 55% by 31 Oct · resets 1 Nov
State: ok
APP   THIS MONTH   BY MONTH END
blog  $0.62        about $0.90
Rates: memory $0.025/GiB-hour · CPU $0.10/vCPU-hour · disk $0.30/GiB-month · data out $0.20/GiB
At 100%, your apps pause and nothing is lost; they start again on 1 Nov, or at once on upgrading to Pro.
"
    );
    assert_eq!(server.paths(), ["GET /v1/usage HTTP/1.1"]);
    let server = Server::start(vec![reply(account())]).await;
    assert_eq!(
        success(&server.run(&["--json", "usage", "--detail"]).await),
        account()
    );
}

#[tokio::test]
async fn app_dollars_and_requested_breakdown_are_added_to_its_usage() {
    let mut value = healthy();
    value["cost"] = json!({"used_microusd": 1230000, "forecast_microusd": 1650000,
        "memory_microusd": 1000000, "cpu_microusd": 90000, "disk_microusd": 90000, "egress_microusd": 50000});
    let server = Server::start(vec![reply(value)]).await;
    let output = server.run(&["usage", "notes", "--detail"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).starts_with(
            "\
notes: up to 1 shared vCPU, 2 GiB memory, 10 GiB disk; running
This month: $1.23 · about $1.65 by month end
  memory $1.00 · CPU $0.09 · disk $0.09 · data out $0.05
last 24 hours, a point every 5 minutes; readings for all of it
"
        ),
        "{}",
        text(&output.stdout)
    );
}

#[tokio::test]
async fn each_state_says_what_happens_at_100_percent_unless_enforcement_is_stopped() {
    for (state, used, percent) in [
        ("warned_75", 2_250_000, 75),
        ("warned_90", 2_700_000, 90),
        ("stopped", 3_000_000, 100),
    ] {
        let mut usage = account();
        usage["state"] = json!(state);
        usage["used_microusd"] = json!(used);
        usage["percent_used"] = json!(percent);
        usage["plan"] = json!("pro");
        let server = Server::start(vec![reply(usage.clone())]).await;
        let output = server.run(&["usage"]).await;
        assert!(output.status.success(), "{}", text(&output.stderr));
        let shown = text(&output.stdout);
        assert!(shown.contains(&format!("State: {state}\n")), "{shown}");
        assert!(
            shown.contains("At 100%, your apps pause and nothing is lost; they start again on 1 Nov, or at once after a top-up from the portal's usage page.\n"),
            "{shown}"
        );
        // An operator's emergency stop: the state says so, and nothing claims
        // a pause.
        usage["enforced"] = json!(false);
        let server = Server::start(vec![reply(usage)]).await;
        let output = server.run(&["usage"]).await;
        assert!(output.status.success(), "{}", text(&output.stderr));
        let shown = text(&output.stdout);
        assert!(
            shown.contains(&format!("State: {state} (enforcement off)\n")),
            "{shown}"
        );
        assert!(!shown.contains("pause"), "{shown}");
    }
}

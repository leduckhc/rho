//! Wire-format tests. See SPEC-jsonl-frontend section 3.
//!
//! Every test here is pure serde. No network, no filesystem, no time.

use rho_core::{AgentStopReason, StopReason, ToolKind};
use rho_jsonl::{
    Command, DialogAnswer, DialogRequest, Event, FaultKind, Reply, ReplyError, SettleReason,
};

fn line<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("the wire type must serialise")
}

#[test]
fn a_success_reply_carries_no_error_field() {
    let reply = Reply::ok("prompt", Some("r1".to_string()));
    let text = line(&reply);
    assert_eq!(text, r#"{"req_id":"r1","command":"prompt","success":true}"#);
    assert!(
        !text.contains("error"),
        "a success reply must carry no error key"
    );
}

#[test]
fn an_error_reply_always_says_success_false() {
    let reply = Reply::err(
        "steer",
        Some("r2".to_string()),
        ReplyError::QueueFull,
        "the steering queue is full",
    );
    let text = line(&reply);
    assert_eq!(
        text,
        r#"{"req_id":"r2","command":"steer","success":false,"error":"queue_full","message":"the steering queue is full"}"#
    );
}

#[test]
fn a_reply_routes_on_the_success_field() {
    // A client routes every line on `success`. Prove the reader does the same, so a
    // third-party Rust client cannot read an error as a success.
    let ok: Reply = serde_json::from_str(r#"{"command":"prompt","success":true}"#).expect("ok");
    assert!(matches!(ok, Reply::Ok(_)));
    let bad: Reply = serde_json::from_str(
        r#"{"command":"prompt","success":false,"error":"parse_error","message":"x"}"#,
    )
    .expect("err");
    assert!(matches!(bad, Reply::Err(_)));

    // The one-value types must refuse the wrong literal, or the two arms could both
    // match and the reader would return the first. See
    // D-two-variants-cannot-share-a-serde-tag.
    let wrong = serde_json::from_str::<Reply>(
        r#"{"command":"prompt","success":true,"error":"parse_error","message":"x"}"#,
    );
    match wrong {
        Ok(Reply::Ok(_)) => {}
        other => panic!("expected the ok arm to win on success:true, got {other:?}"),
    }
}

#[test]
fn reply_carries_req_id() {
    let with = line(&Reply::ok("abort", Some("r9".to_string())));
    assert!(with.contains(r#""req_id":"r9""#));
    // A command with no req_id gets a reply with no req_id key at all, not a null.
    let without = line(&Reply::ok("abort", None));
    assert!(
        !without.contains("req_id"),
        "an absent req_id must not serialise as null: {without}"
    );
}

#[test]
fn an_event_has_a_type_and_no_success_field() {
    // A client routes each line on two fields: `type` for an event, `success` for a
    // reply. Every event must carry the first and none may carry the second.
    let events = vec![
        Event::TurnStart,
        Event::TurnEnd {
            stop_reason: StopReason::EndTurn,
        },
        Event::TextDelta {
            index: 0,
            delta: "hi".to_string(),
        },
        Event::ToolStart {
            id: "t1".to_string(),
            name: "read".to_string(),
            kind: ToolKind::Read,
        },
        Event::ToolUpdate {
            id: "t1".to_string(),
            output: "a line".to_string(),
        },
        Event::ToolEnd {
            id: "t1".to_string(),
            ok: true,
        },
        Event::MessageQueued { position: 1 },
        Event::MessageDelivered { count: 2 },
        Event::Dialog(DialogRequest::Notify {
            id: "d0".to_string(),
            message: "hello".to_string(),
        }),
        Event::Fault {
            kind: FaultKind::Provider,
            message: "boom".to_string(),
        },
        Event::Settled {
            stop_reason: SettleReason::EndTurn,
        },
    ];
    for event in &events {
        let text = line(event);
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert!(
            value.get("type").and_then(|t| t.as_str()).is_some(),
            "every event needs a type: {text}"
        );
        assert!(
            value.get("success").is_none(),
            "an event must not carry success, or a client cannot route it: {text}"
        );
        let back: Event = serde_json::from_str(&text).expect("an event must round trip");
        assert_eq!(&back, event);
    }
}

#[test]
fn every_settle_reason_has_a_wire_value() {
    // The map from the core enum is exhaustive with no wildcard arm, so this list
    // fails to compile when rho-core gains a stop reason. That is the point.
    let pairs = [
        (AgentStopReason::EndTurn, "end_turn"),
        (AgentStopReason::MaxTokens, "max_tokens"),
        (AgentStopReason::MaxToolCalls, "max_tool_calls"),
        (AgentStopReason::MaxTurnRequests, "max_turn_requests"),
        (AgentStopReason::Refusal, "refusal"),
        // The ACP spelling, with two letter l. Rust spells the variant with one.
        (AgentStopReason::Canceled, "cancelled"),
    ];
    let mut seen: Vec<SettleReason> = Vec::new();
    for (reason, wire) in pairs {
        let mapped = SettleReason::from(reason);
        let text = line(&Event::Settled {
            stop_reason: mapped,
        });
        assert_eq!(
            text,
            format!(r#"{{"type":"settled","stop_reason":"{wire}"}}"#)
        );
        assert!(
            !seen.contains(&mapped),
            "two stop reasons mapped to {mapped:?}"
        );
        seen.push(mapped);
    }
    // `faulted` is this crate's own case. It must not collide with a core value.
    assert!(!seen.contains(&SettleReason::Faulted));
    assert_eq!(
        line(&Event::Settled {
            stop_reason: SettleReason::Faulted
        }),
        r#"{"type":"settled","stop_reason":"faulted"}"#
    );
}

#[test]
fn every_turn_end_stop_reason_has_a_wire_value() {
    // `TurnEnd` carries rho-core's `StopReason`, which is a different enum from the
    // run-level one. A client reads both, so both need pinned wire values.
    let pairs = [
        (StopReason::EndTurn, "end_turn"),
        (StopReason::ToolUse, "tool_use"),
        (StopReason::MaxTokens, "max_tokens"),
        (StopReason::StopSequence, "stop_sequence"),
        (StopReason::ContentFiltered, "content_filtered"),
        (StopReason::Canceled, "canceled"),
    ];
    for (reason, wire) in pairs {
        assert_eq!(
            line(&Event::TurnEnd {
                stop_reason: reason
            }),
            format!(r#"{{"type":"turn_end","stop_reason":"{wire}"}}"#)
        );
    }
}

#[test]
fn every_tool_kind_reaches_the_wire() {
    // `kind` is rho-core's own enum, not a copy, so this test proves the wire value
    // of each variant rather than a mapping. A copy had already drifted before either
    // side existed. See D-the-wire-reuses-the-core-stop-reason.
    let pairs = [
        (ToolKind::Read, "read"),
        (ToolKind::Edit, "edit"),
        (ToolKind::Delete, "delete"),
        (ToolKind::Move, "move"),
        (ToolKind::Search, "search"),
        (ToolKind::Execute, "execute"),
        (ToolKind::Think, "think"),
        (ToolKind::Fetch, "fetch"),
        (ToolKind::SwitchMode, "switch_mode"),
        (ToolKind::Other, "other"),
    ];
    for (kind, wire) in pairs {
        let text = line(&Event::ToolStart {
            id: "t".to_string(),
            name: "x".to_string(),
            kind,
        });
        assert!(
            text.contains(&format!(r#""kind":"{wire}""#)),
            "expected kind {wire} in {text}"
        );
    }
}

#[test]
fn unknown_command_is_reply_error() {
    // pi has 35 commands and rho starts with nine, so a client will meet this.
    let error = serde_json::from_str::<Command>(r#"{"type":"compact"}"#)
        .expect_err("an unknown type must not parse");
    assert!(
        error.to_string().starts_with("unknown variant"),
        "the serve loop routes on this prefix: {error}"
    );
}

#[test]
fn parse_error_is_reply_error() {
    assert!(serde_json::from_str::<Command>("{not json").is_err());
    // A missing required field is a parse error, not a command with a default.
    assert!(serde_json::from_str::<Command>(r#"{"type":"prompt"}"#).is_err());
}

#[test]
fn an_unknown_field_on_a_command_is_refused() {
    // A command is an instruction, and an unknown field may be the part that limits
    // it. So rho refuses the whole line. See
    // D-a-command-is-strict-and-an-event-is-loose.
    let error =
        serde_json::from_str::<Command>(r#"{"type":"prompt","message":"hi","only_if_cheap":true}"#)
            .expect_err("an unknown field must be refused");
    assert!(
        error.to_string().contains("only_if_cheap"),
        "the message must name the field: {error}"
    );
}

#[test]
fn a_new_field_on_an_event_is_ignored_by_an_old_reader() {
    // The other half of the same contract. An event is a report, so a client that
    // does not know a field ignores it and shows less, rather than failing.
    let event: Event =
        serde_json::from_str(r#"{"type":"settled","stop_reason":"end_turn","tokens_used":1234}"#)
            .expect("an event reader must ignore an unknown field");
    assert_eq!(
        event,
        Event::Settled {
            stop_reason: SettleReason::EndTurn
        }
    );
}

#[test]
fn an_absent_req_id_reads_as_none() {
    let command: Command = serde_json::from_str(r#"{"type":"abort"}"#).expect("req_id is optional");
    assert_eq!(command.req_id(), None);
    assert_eq!(command.name(), "abort");
}

#[test]
fn get_commands_lists_every_command() {
    // The list is what `get_commands` reports, so a client discovers the command set.
    // Build one of every variant, and assert its name is in the list. A new variant
    // with no entry fails here.
    let all = vec![
        Command::Prompt {
            req_id: None,
            message: String::new(),
        },
        Command::Steer {
            req_id: None,
            message: String::new(),
        },
        Command::Abort { req_id: None },
        Command::GetState { req_id: None },
        Command::SetModel {
            req_id: None,
            provider: String::new(),
            model_id: String::new(),
        },
        Command::NewSession { req_id: None },
        Command::GetMessages { req_id: None },
        Command::GetCommands { req_id: None },
        Command::DialogResponse {
            req_id: None,
            id: String::new(),
            answer: DialogAnswer::Cancelled,
        },
    ];
    assert_eq!(
        all.len(),
        Command::NAMES.len(),
        "NAMES must hold one entry per Command variant"
    );
    for command in &all {
        assert!(
            Command::NAMES.contains(&command.name()),
            "{} is missing from Command::NAMES",
            command.name()
        );
    }
    // Every name must also parse back as a command type, so the list cannot name a
    // command the reader does not accept.
    for name in Command::NAMES {
        let probe = format!(r#"{{"type":"{name}"}}"#);
        let error = serde_json::from_str::<Command>(&probe).err();
        if let Some(error) = error {
            assert!(
                !error.to_string().starts_with("unknown variant"),
                "{name} is advertised but the reader rejects the type"
            );
        }
    }
}

#[test]
fn a_dialog_answer_holds_exactly_one_value() {
    // The wire shape, both ways.
    let cases = [
        (DialogAnswer::Value("a".to_string()), r#"{"value":"a"}"#),
        (DialogAnswer::Confirmed(true), r#"{"confirmed":true}"#),
        (DialogAnswer::Confirmed(false), r#"{"confirmed":false}"#),
        (DialogAnswer::Cancelled, r#"{"cancelled":true}"#),
    ];
    for (answer, wire) in cases {
        assert_eq!(line(&answer), wire);
        let back: DialogAnswer = serde_json::from_str(wire).expect("round trip");
        assert_eq!(back, answer);
    }
}

#[test]
fn a_dialog_answer_with_two_keys_is_invalid_argument() {
    // An untagged reader would return the first match in silence, and a dialog answer
    // decides whether a tool runs. See D-a-dialog-answer-holds-exactly-one-value.
    let error = serde_json::from_str::<DialogAnswer>(r#"{"confirmed":false,"cancelled":true}"#)
        .expect_err("two answers in one object must be refused");
    assert!(
        error.to_string().contains("exactly one"),
        "the message must say the rule: {error}"
    );
    // And the same through a whole command, which is how it really arrives.
    assert!(
        serde_json::from_str::<Command>(
            r#"{"type":"dialog_response","id":"d1","answer":{"value":"yes","cancelled":true}}"#
        )
        .is_err()
    );
}

#[test]
fn a_dialog_answer_with_no_key_is_invalid_argument() {
    let error =
        serde_json::from_str::<DialogAnswer>("{}").expect_err("an empty answer must be refused");
    assert!(error.to_string().contains("exactly one"), "{error}");
}

#[test]
fn a_dialog_request_round_trips_under_two_tags() {
    // One line carries `type` for the event and `method` for the dialog. Prove both
    // survive, because a nested tagged enum is where serde surprises live.
    let requests = vec![
        DialogRequest::Select {
            id: "d1".to_string(),
            title: "pick".to_string(),
            options: vec!["a".to_string(), "b".to_string()],
            timeout_ms: Some(500),
        },
        DialogRequest::Confirm {
            id: "d2".to_string(),
            title: "sure?".to_string(),
            message: "it deletes a file".to_string(),
            timeout_ms: None,
        },
        DialogRequest::Input {
            id: "d3".to_string(),
            title: "name".to_string(),
            placeholder: Some("type here".to_string()),
            timeout_ms: Some(10),
        },
        DialogRequest::Notify {
            id: "d4".to_string(),
            message: "done".to_string(),
        },
    ];
    for request in requests {
        let expects_block = !matches!(request, DialogRequest::Notify { .. });
        assert_eq!(request.blocks(), expects_block);
        let event = Event::Dialog(request.clone());
        let text = line(&event);
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["type"], "dialog");
        assert!(
            value.get("method").is_some(),
            "the method tag is lost: {text}"
        );
        assert_eq!(value["id"], request.id());
        let back: Event = serde_json::from_str(&text).expect("round trip");
        assert_eq!(back, event);
    }
}

#[test]
fn a_line_holding_a_newline_in_a_string_is_escaped() {
    // A prompt with a newline must not split into two records. serde_json escapes it,
    // and this test pins that, because the framing rule depends on it.
    let command = Command::Prompt {
        req_id: None,
        message: "first\nsecond".to_string(),
    };
    let text = line(&command);
    assert!(
        !text.contains('\n'),
        "a record must hold no raw newline: {text}"
    );
    assert!(text.contains("\\n"));
    let back: Command = serde_json::from_str(&text).expect("round trip");
    assert_eq!(back, command);
}

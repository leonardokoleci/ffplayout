use super::*;

async fn get_queue(fixture: &SecurityFixture, token: &str) -> serde_json::Value {
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "GET",
            "/api/rundown/1",
            Some(token),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap()
}

fn raw_request(
    method: &str,
    uri: &str,
    token: Option<&str>,
    content_type: &str,
    body: &str,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", content_type);

    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }

    request.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
#[serial]
async fn rundown_queue_import_accepts_json_and_m3u() {
    let fixture = security_fixture().await;
    let user = access_token(&fixture.state, 1).await;

    // Without a token the import is rejected.
    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            None,
            "application/json",
            r#"{"program": [{"source": "rtmp://live.example/one"}]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Engine playlist format.
    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            Some(&user),
            "application/json",
            r#"{"channel":"Channel 1","date":"2026-11-19","program":[
                {"source": "rtmp://live.example/one"},
                {"source": "rtmp://live.example/two"}]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let queue: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(queue["items"].as_array().unwrap().len(), 2);

    // Bare item array.
    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            Some(&user),
            "application/json",
            r#"[{"source": "rtmp://live.example/one", "out": 45.0}]"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let queue: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(queue["items"].as_array().unwrap().len(), 1);
    assert_eq!(queue["items"][0]["out"], json!(45.0));

    // M3U with an ffplayout trim tag: seek/out round-trip, EXTINF duration
    // becomes the out point, titles carry over.
    let m3u = "#EXTM3U\n\
        #EXTINF:30.000,Cool Clip\n\
        #EXTFFP:5.000,35.000\n\
        rtmp://live.example/one\n\
        #EXTINF:60,Second\n\
        rtmp://live.example/two\n";

    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            Some(&user),
            "text/plain",
            m3u,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let queue: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    let items = queue["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["title"], json!("Cool Clip"));
    assert_eq!(items[0]["in"], json!(5.0));
    assert_eq!(items[0]["out"], json!(35.0));
    assert_eq!(items[1]["out"], json!(60.0));

    // Invalid JSON is rejected.
    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            Some(&user),
            "application/json",
            "{oops",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // A real probed source with an EXTINF duration rounded slightly above
    // the natural media duration is clamped, not rejected.
    let asset = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests_assets/storage/media_sorted/DarkGray_00-00-10.mp4")
        .to_string_lossy()
        .replace('\\', "/");

    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            Some(&user),
            "text/plain",
            &format!("#EXTM3U\n#EXTINF:10.001,Rounded\n{asset}\n"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let queue: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    let items = queue["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        items[0]["out"].as_f64().unwrap() <= 10.001,
        "out is clamped to the media duration"
    );

    // An out point far beyond the media duration stays an error.
    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            Some(&user),
            "text/plain",
            &format!("#EXTM3U\n#EXTINF:30.000,TooLong\n{asset}\n"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // An empty M3U clears the queue.
    let response = fixture
        .app
        .clone()
        .oneshot(raw_request(
            "POST",
            "/api/rundown/1/queue",
            Some(&user),
            "text/plain",
            "#EXTM3U\n",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let queue = get_queue(&fixture, &user).await;
    assert_eq!(queue["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
#[serial]
async fn rundown_queue_crud_cursor_and_authorization() {
    let fixture = security_fixture().await;
    let user = access_token(&fixture.state, 1).await;

    for (channel, token, expected) in [
        (1, None, StatusCode::UNAUTHORIZED),
        (2, Some(user.as_str()), StatusCode::FORBIDDEN),
        (1, Some(user.as_str()), StatusCode::OK),
    ] {
        let response = fixture
            .app
            .clone()
            .oneshot(json_request(
                "GET",
                &format!("/api/rundown/{channel}"),
                token,
                json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }

    // Live sources are accepted without an ffprobe run.
    for source in ["rtmp://live.example/one", "rtmp://live.example/two"] {
        let response = fixture
            .app
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/rundown/1/item",
                Some(&user),
                json!({"source": source}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let queue = get_queue(&fixture, &user).await;
    assert_eq!(queue["items"].as_array().unwrap().len(), 2);
    assert_eq!(queue["items"][0]["position"], 0);
    assert_eq!(queue["items"][0]["duration"], json!(60.0));
    assert_eq!(queue["items"][1]["position"], 1);

    let one_id = queue["items"][0]["id"].as_i64().unwrap();
    let two_id = queue["items"][1]["id"].as_i64().unwrap();

    // Unstarted queue: items are previewed back-to-back from now.
    let begin0 = queue["items"][0]["begin_epoch"].as_f64().unwrap();
    let begin1 = queue["items"][1]["begin_epoch"].as_f64().unwrap();
    assert!((begin1 - begin0 - 60.0).abs() < 1.0);

    // A local source that cannot be probed is rejected.
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/rundown/1/item",
            Some(&user),
            json!({"source": "/definitely/not/here.mp4"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Trim the second item's out point.
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/rundown/1/item/{two_id}"),
            Some(&user),
            json!({"out": 30.0}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let trimmed: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(trimmed["out"], json!(30.0));

    // Move the second item to the top.
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/rundown/1/move",
            Some(&user),
            json!({"item_id": two_id, "after_position": -1}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let moved: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(moved["items"][0]["id"], json!(two_id));

    // Jump back to the first item; it becomes the cursor head and is marked
    // for takeover, so the playout plays it from its beginning.
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/rundown/1/jump",
            Some(&user),
            json!({"item_id": one_id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let queue = get_queue(&fixture, &user).await;
    assert_eq!(queue["head_item_id"], json!(one_id));

    let state: (bool, bool) =
        sqlx::query_as("SELECT takeover, drained FROM rundown_state WHERE channel_id = 1")
            .fetch_one(&fixture.state.pool)
            .await
            .unwrap();
    assert!(state.0, "jump marks the head for takeover");
    assert!(!state.1);

    // Jumping to the head item itself restarts it instead of skipping forward.
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/rundown/1/jump",
            Some(&user),
            json!({"item_id": one_id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let queue = get_queue(&fixture, &user).await;
    assert_eq!(queue["head_item_id"], json!(one_id));

    // Deleting the head advances to the successor.
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "DELETE",
            &format!("/api/rundown/1/item/{one_id}"),
            Some(&user),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let queue = get_queue(&fixture, &user).await;
    assert_eq!(queue["head_item_id"], json!(two_id));
    assert_eq!(queue["items"].as_array().unwrap().len(), 1);

    // Deleting the last item parks the cursor on an empty queue.
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "DELETE",
            &format!("/api/rundown/1/item/{two_id}"),
            Some(&user),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let queue = get_queue(&fixture, &user).await;
    assert!(queue["head_item_id"].is_null());
    assert_eq!(queue["items"].as_array().unwrap().len(), 0);
}

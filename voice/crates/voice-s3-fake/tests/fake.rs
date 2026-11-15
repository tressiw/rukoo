//! The fake S3 over real HTTP: each fault does what its comment says, plus routing, auth,
//! ranges, multipart and the on-disk folder.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use voice_s3_fake::{FakeS3, Op};

const CREDENTIAL: &str = "X-Amz-Credential=test-key%2F20261102%2Fus-east-1%2Fs3%2Faws4_request";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "voice-s3-fake-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn start() -> FakeS3 {
    FakeS3::start("test", "test-key", None).unwrap()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into()
}

/// A path-style URL for `key` with an access key and `query`.
fn url(fake: &FakeS3, key: &str, query: &str) -> String {
    let sep = if query.is_empty() { "" } else { "&" };
    format!("{}/test/{key}?{CREDENTIAL}{sep}{query}", fake.endpoint())
}

struct Got {
    status: u16,
    body: Vec<u8>,
    etag: Option<String>,
    range: Option<String>,
}

fn got(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Got, ureq::Error> {
    let mut response = result?;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let etag = header("etag");
    let range = header("content-range");
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_vec()?;
    Ok(Got {
        status,
        body,
        etag,
        range,
    })
}

fn put(fake: &FakeS3, key: &str, body: &[u8]) -> Result<Got, ureq::Error> {
    got(agent().put(url(fake, key, "")).send(body))
}

fn get(fake: &FakeS3, key: &str) -> Got {
    got(agent().get(url(fake, key, "")).call()).unwrap()
}

fn text(got: &Got) -> String {
    String::from_utf8_lossy(&got.body).into_owned()
}

fn create(fake: &FakeS3, key: &str) -> String {
    let reply = got(agent().post(url(fake, key, "uploads")).send_empty()).unwrap();
    assert_eq!(reply.status, 200);
    let body = text(&reply);
    let start = body.find("<UploadId>").unwrap() + "<UploadId>".len();
    let end = body.find("</UploadId>").unwrap();
    body[start..end].to_string()
}

fn upload_part(fake: &FakeS3, key: &str, id: &str, number: u16, body: &[u8]) -> Got {
    got(agent()
        .put(url(
            fake,
            key,
            &format!("partNumber={number}&uploadId={id}"),
        ))
        .send(body))
    .unwrap()
}

fn complete(fake: &FakeS3, key: &str, id: &str, etags: &[String]) -> Got {
    let mut body = String::from("<CompleteMultipartUpload>");
    for (i, etag) in etags.iter().enumerate() {
        body.push_str(&format!(
            "<Part><ETag>{}</ETag><PartNumber>{}</PartNumber></Part>",
            etag.replace('"', "&quot;"),
            i + 1
        ));
    }
    body.push_str("</CompleteMultipartUpload>");
    got(agent()
        .post(url(fake, key, &format!("uploadId={id}")))
        .send(body.as_bytes()))
    .unwrap()
}

/// A request written by hand, for headers ureq sets itself.
fn raw(fake: &FakeS3, request: &str) -> String {
    let address = fake.endpoint().trim_start_matches("http://").to_string();
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

#[test]
fn objects_go_in_and_come_out() {
    let fake = start();
    let reply = put(&fake, "a/b.bin", b"0123456789").unwrap();
    assert_eq!(reply.status, 200);
    let etag = reply.etag.unwrap();
    assert!(
        etag.starts_with("\"a-") && etag.ends_with('"') && etag.len() == 20,
        "{etag}"
    );

    let reply = get(&fake, "a/b.bin");
    assert_eq!(
        (reply.status, reply.body.as_slice()),
        (200, &b"0123456789"[..])
    );
    assert_eq!(reply.etag.as_deref(), Some(etag.as_str()));
    let head = agent().head(url(&fake, "a/b.bin", "")).call().unwrap();
    assert_eq!(head.status().as_u16(), 200);
    assert_eq!(head.headers().get("content-length").unwrap(), "10");
    assert_eq!(
        fake.objects(),
        vec![("a/b.bin".to_string(), b"0123456789".to_vec())]
    );

    let missing = get(&fake, "nope");
    assert_eq!(missing.status, 404);
    assert!(text(&missing).contains("<Code>NoSuchKey</Code>"));

    let reply = got(agent().delete(url(&fake, "a/b.bin", "")).call()).unwrap();
    assert_eq!(reply.status, 204);
    assert!(fake.objects().is_empty());

    let ops: Vec<(Op, u16)> = fake.log().iter().map(|l| (l.op, l.status)).collect();
    assert_eq!(
        ops,
        [
            (Op::Put, 200),
            (Op::Get, 200),
            (Op::Head, 200),
            (Op::Get, 404),
            (Op::Delete, 204)
        ]
    );
    let log = fake.log();
    assert_eq!(log[0].bytes, 10);
    assert_eq!(log[0].key, "a/b.bin");
    assert_eq!(log[1].bytes, 10);
    assert!(log[0].at > 1_700_000_000_000 && log[0].at <= log[4].at);
}

#[test]
fn ranges_follow_http() {
    let fake = start();
    put(&fake, "r", b"0123456789").unwrap();
    let range = |value: &str| {
        got(agent()
            .get(url(&fake, "r", ""))
            .header("Range", value)
            .call())
        .unwrap()
    };
    let reply = range("bytes=2-4");
    assert_eq!((reply.status, reply.body.as_slice()), (206, &b"234"[..]));
    assert_eq!(reply.range.as_deref(), Some("bytes 2-4/10"));
    assert_eq!(range("bytes=7-").body, b"789");
    assert_eq!(range("bytes=-2").body, b"89");
    assert_eq!(range("bytes=8-100").body, b"89");
    let refused = range("bytes=10-");
    assert_eq!(refused.status, 416);
    assert_eq!(refused.range.as_deref(), Some("bytes */10"));
}

#[test]
fn requests_need_the_access_key_and_an_unexpired_url() {
    let fake = start();
    put(&fake, "k", b"x").unwrap();
    let call = |url: String| got(agent().get(url).call()).unwrap();
    let base = format!("{}/test/k", fake.endpoint());

    let anonymous = call(base.clone());
    assert_eq!(anonymous.status, 403);
    assert!(text(&anonymous).contains("<Code>AccessDenied</Code>"));

    let wrong = call(format!(
        "{base}?X-Amz-Credential=other%2F20261102%2Fus-east-1%2Fs3%2Faws4_request"
    ));
    assert_eq!(wrong.status, 403);
    assert!(text(&wrong).contains("<Code>InvalidAccessKeyId</Code>"));

    let expired = call(format!(
        "{base}?{CREDENTIAL}&X-Amz-Date=20200101T000000Z&X-Amz-Expires=900&X-Amz-Signature=00"
    ));
    assert_eq!(expired.status, 403);
    assert!(text(&expired).contains("<Code>AccessDenied</Code>"));
    assert!(text(&expired).contains("Request has expired"));

    let valid = call(format!(
        "{base}?{CREDENTIAL}&X-Amz-Date=20990101T000000Z&X-Amz-Expires=900&X-Amz-Signature=00"
    ));
    assert_eq!(valid.status, 200);

    // The header form of the credential works too.
    let header = got(agent()
        .get(&base)
        .header(
            "Authorization",
            "AWS4-HMAC-SHA256 Credential=test-key/20261102/us-east-1/s3/aws4_request, SignedHeaders=host, Signature=00",
        )
        .call())
    .unwrap();
    assert_eq!(header.status, 200);

    let other_bucket = call(format!("{}/other/k?{CREDENTIAL}", fake.endpoint()));
    assert_eq!(other_bucket.status, 404);
    assert!(text(&other_bucket).contains("<Code>NoSuchBucket</Code>"));
}

#[test]
fn virtual_host_requests_name_the_bucket_in_the_host() {
    let fake = start();
    put(&fake, "dir/v", b"virtual").unwrap();
    let port = fake.endpoint().rsplit(':').next().unwrap().to_string();
    let answer = raw(
        &fake,
        &format!(
            "GET /dir/v?{CREDENTIAL} HTTP/1.1\r\nHost: test.localhost:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.ends_with("virtual"));
    let answer = raw(
        &fake,
        &format!(
            "GET /test/dir/v?{CREDENTIAL} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
}

#[test]
fn fail_answers_the_next_matching_requests_once_each() {
    let fake = start();
    fake.faults().fail = vec![(Op::Put, 503, "SlowDown"), (Op::Put, 500, "InternalError")];
    // Another op isn't affected.
    assert_eq!(get(&fake, "none").status, 404);
    let first = put(&fake, "k", b"1").unwrap();
    assert_eq!(first.status, 503);
    assert!(text(&first).contains("<Code>SlowDown</Code>"));
    let second = put(&fake, "k", b"2").unwrap();
    assert_eq!(second.status, 500);
    assert!(text(&second).contains("<Code>InternalError</Code>"));
    assert!(fake.objects().is_empty());
    assert_eq!(put(&fake, "k", b"3").unwrap().status, 200);
    assert!(fake.faults().fail.is_empty());
    let statuses: Vec<u16> = fake
        .log()
        .iter()
        .filter(|l| l.op == Op::Put)
        .map(|l| l.status)
        .collect();
    assert_eq!(statuses, [503, 500, 200]);
}

#[test]
fn stall_holds_every_answer() {
    let fake = start();
    fake.faults().stall = Some(Duration::from_millis(400));
    let started = Instant::now();
    assert_eq!(put(&fake, "k", b"x").unwrap().status, 200);
    assert_eq!(get(&fake, "k").status, 200);
    assert!(started.elapsed() >= Duration::from_millis(800));
    fake.faults().stall = None;
    let started = Instant::now();
    assert_eq!(get(&fake, "k").status, 200);
    assert!(started.elapsed() < Duration::from_millis(400));
}

#[test]
fn drop_after_closes_the_next_upload_midway() {
    let fake = start();
    fake.faults().drop_after = Some(10);
    let body = vec![7u8; 100_000];
    assert!(put(&fake, "k", &body).is_err());
    assert!(fake.objects().is_empty());
    assert!(
        fake.log().is_empty(),
        "a dropped request has no answer to log"
    );
    assert_eq!(fake.faults().drop_after, None);
    assert_eq!(put(&fake, "k", &body).unwrap().status, 200);
    assert_eq!(fake.objects()[0].1.len(), 100_000);

    // A part is dropped the same way and isn't kept.
    let id = create(&fake, "big");
    fake.faults().drop_after = Some(1);
    assert!(got(agent()
        .put(url(&fake, "big", &format!("partNumber=1&uploadId={id}")))
        .send(&body[..]))
    .is_err());
    let parts = got(agent()
        .get(url(&fake, "big", &format!("uploadId={id}")))
        .call())
    .unwrap();
    assert!(!text(&parts).contains("<Part>"), "{}", text(&parts));
}

#[test]
fn refuse_closes_every_connection() {
    let fake = start();
    put(&fake, "k", b"x").unwrap();
    fake.faults().refuse = true;
    assert!(got(agent().get(url(&fake, "k", "")).call()).is_err());
    assert!(put(&fake, "k", b"changed").is_err());
    fake.faults().refuse = false;
    assert_eq!(get(&fake, "k").body, b"x");
}

#[test]
fn r2_parts_refuses_unequal_non_last_parts() {
    let fake = start();
    let upload = |sizes: &[usize]| {
        let id = create(&fake, "obj");
        let etags: Vec<String> = sizes
            .iter()
            .enumerate()
            .map(|(i, &size)| {
                let reply = upload_part(&fake, "obj", &id, i as u16 + 1, &vec![i as u8; size]);
                assert_eq!(reply.status, 200);
                reply.etag.unwrap()
            })
            .collect();
        complete(&fake, "obj", &id, &etags)
    };
    assert_eq!(upload(&[5, 4, 3]).status, 200);
    fake.faults().r2_parts = true;
    let refused = upload(&[5, 4, 3]);
    assert_eq!(refused.status, 400);
    assert!(text(&refused).contains("<Code>InvalidPart</Code>"));
    assert_eq!(upload(&[5, 5, 6]).status, 400);
    assert_eq!(upload(&[5, 5, 3]).status, 200);
    assert_eq!(upload(&[5]).status, 200);
    assert_eq!(fake.objects()[0].1, vec![0u8; 5]);
}

#[test]
fn forget_uploads_answers_no_such_upload_for_open_ids() {
    let fake = start();
    let old = create(&fake, "obj");
    assert_eq!(upload_part(&fake, "obj", &old, 1, b"abc").status, 200);
    fake.faults().forget_uploads = true;
    let reply = upload_part(&fake, "obj", &old, 2, b"def");
    assert_eq!(reply.status, 404);
    assert!(text(&reply).contains("<Code>NoSuchUpload</Code>"));
    let listed = got(agent()
        .get(url(&fake, "obj", &format!("uploadId={old}")))
        .call())
    .unwrap();
    assert_eq!(listed.status, 404);
    // A new upload works, and completes.
    let new = create(&fake, "obj");
    let etag = upload_part(&fake, "obj", &new, 1, b"xyz").etag.unwrap();
    assert_eq!(complete(&fake, "obj", &new, &[etag]).status, 200);
    assert_eq!(fake.objects()[0].1, b"xyz");
}

#[test]
fn multipart_uploads_list_complete_and_abort() {
    let fake = start();
    let id = create(&fake, "m/obj");
    let mut etags = Vec::new();
    for (n, body) in [(1u16, &b"aaaa"[..]), (2, b"bbbb"), (3, b"cc")] {
        etags.push(upload_part(&fake, "m/obj", &id, n, body).etag.unwrap());
    }
    let listed = text(
        &got(agent()
            .get(url(&fake, "m/obj", &format!("uploadId={id}&max-parts=2")))
            .call())
        .unwrap(),
    );
    assert!(
        listed.contains("<IsTruncated>true</IsTruncated>"),
        "{listed}"
    );
    assert!(listed.contains("<NextPartNumberMarker>2</NextPartNumberMarker>"));
    let rest = text(
        &got(agent()
            .get(url(
                &fake,
                "m/obj",
                &format!("uploadId={id}&part-number-marker=2"),
            ))
            .call())
        .unwrap(),
    );
    assert!(
        rest.contains("<PartNumber>3</PartNumber>") && !rest.contains("<PartNumber>2</PartNumber>")
    );

    // Out of order and a wrong ETag are refused.
    let wrong = complete(&fake, "m/obj", &id, &[etags[1].clone(), etags[0].clone()]);
    assert_eq!(wrong.status, 400);
    let done = complete(&fake, "m/obj", &id, &etags);
    assert_eq!(done.status, 200, "{}", text(&done));
    assert!(text(&done).contains("<CompleteMultipartUploadResult"));
    assert_eq!(get(&fake, "m/obj").body, b"aaaabbbbcc");
    // The upload is gone once complete.
    assert_eq!(complete(&fake, "m/obj", &id, &etags).status, 404);

    let id = create(&fake, "m/other");
    upload_part(&fake, "m/other", &id, 1, b"zz");
    let aborted = got(agent()
        .delete(url(&fake, "m/other", &format!("uploadId={id}")))
        .call())
    .unwrap();
    assert_eq!(aborted.status, 204);
    let again = got(agent()
        .delete(url(&fake, "m/other", &format!("uploadId={id}")))
        .call())
    .unwrap();
    assert_eq!(again.status, 404);
    assert_eq!(fake.objects().len(), 1);
    let parts: Vec<Option<u16>> = fake
        .log()
        .iter()
        .filter(|l| l.op == Op::UploadPart)
        .map(|l| l.part)
        .collect();
    assert_eq!(parts, [Some(1), Some(2), Some(3), Some(1)]);
}

#[test]
fn a_folder_keeps_objects_across_a_restart_on_the_same_port() {
    let dir = Scratch::new("restart");
    let fake = FakeS3::start("test", "test-key", Some(dir.path().to_path_buf())).unwrap();
    let endpoint = fake.endpoint();
    let port: u16 = endpoint.rsplit(':').next().unwrap().parse().unwrap();
    put(&fake, "rkv1/a/b", b"RKV1 one").unwrap();
    put(&fake, "rkv1/c", b"RKV1 two").unwrap();
    let id = create(&fake, "rkv1/big");
    upload_part(&fake, "rkv1/big", &id, 1, b"part one");
    // A key with characters no file name may hold is refused, not stored elsewhere.
    assert_eq!(put(&fake, "bad%3Aname", b"x").unwrap().status, 400);
    fake.stop();

    // objects/ holds only object bytes, in folders by key.
    let objects = dir.path().join("objects");
    assert_eq!(
        std::fs::read(objects.join("rkv1").join("a").join("b")).unwrap(),
        b"RKV1 one"
    );
    assert_eq!(
        std::fs::read(objects.join("rkv1").join("c")).unwrap(),
        b"RKV1 two"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("uploads").join(&id).join("key")).unwrap(),
        "rkv1/big"
    );

    let fake = FakeS3::start_on(port, "test", "test-key", Some(dir.path().to_path_buf())).unwrap();
    assert_eq!(fake.endpoint(), endpoint);
    let keys: Vec<String> = fake.objects().into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, ["rkv1/a/b", "rkv1/c"]);
    assert_eq!(get(&fake, "rkv1/a/b").body, b"RKV1 one");
    let etag = upload_part(&fake, "rkv1/big", &id, 2, b"two").etag.unwrap();
    let listed = text(
        &got(agent()
            .get(url(&fake, "rkv1/big", &format!("uploadId={id}")))
            .call())
        .unwrap(),
    );
    let first = listed
        .split("<ETag>")
        .nth(1)
        .and_then(|s| s.split("</ETag>").next())
        .unwrap()
        .replace("&quot;", "\"");
    assert_eq!(complete(&fake, "rkv1/big", &id, &[first, etag]).status, 200);
    assert_eq!(get(&fake, "rkv1/big").body, b"part onetwo");
    assert!(!dir.path().join("uploads").join(&id).exists());

    got(agent().delete(url(&fake, "rkv1/a/b", "")).call()).unwrap();
    assert!(!objects.join("rkv1").join("a").exists(), "empty folders go");
    fake.stop();

    // One JSON line per answered request, across both runs.
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 11, "{log}");
    assert!(lines[0].starts_with(
        "{\"op\":\"put\",\"key\":\"rkv1/a/b\",\"part\":null,\"bytes\":8,\"status\":200,\"at\":"
    ));
    assert!(lines[3].starts_with("{\"op\":\"upload_part\",\"key\":\"rkv1/big\",\"part\":1,"));
    assert!(log.contains("\"op\":\"list_parts\""));
    assert!(log.contains("\"op\":\"complete\""));
    assert!(lines.iter().all(|l| l.starts_with('{') && l.ends_with('}')));
}

/// Stopping waits for an answer already on its way: the client that gets it finds its line when
/// `stop` returns, which is what reading `log.jsonl` right after `stop` relies on.
#[test]
fn stopping_waits_for_an_answer_on_its_way() {
    let dir = Scratch::new("stop-answer");
    let fake = FakeS3::start("test", "test-key", Some(dir.path().to_path_buf())).unwrap();
    // The answer waits half a second; the put is handled and stored before that wait begins.
    fake.faults().stall = Some(Duration::from_millis(500));
    let target = url(&fake, "rkv1/late", "");
    let sent = std::thread::spawn(move || got(agent().put(target).send(b"late")));
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.objects().is_empty() {
        assert!(Instant::now() < deadline, "the fake never took the put");
        std::thread::sleep(Duration::from_millis(1));
    }
    fake.stop();
    assert_eq!(sent.join().unwrap().unwrap().status, 200);
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");
}

#[test]
fn listings_page_and_group() {
    let fake = start();
    for key in ["p/a/1", "p/a/2", "p/b/1", "p/c", "q/1"] {
        put(&fake, key, b"x").unwrap();
    }
    let list = |query: &str| {
        text(
            &got(agent()
                .get(format!(
                    "{}/test?list-type=2&{CREDENTIAL}&{query}",
                    fake.endpoint()
                ))
                .call())
            .unwrap(),
        )
    };
    let grouped = list("prefix=p%2F&delimiter=%2F");
    assert!(grouped.contains("<CommonPrefixes><Prefix>p/a/</Prefix></CommonPrefixes>"));
    assert!(grouped.contains("<CommonPrefixes><Prefix>p/b/</Prefix></CommonPrefixes>"));
    assert!(grouped.contains("<Key>p/c</Key>"));
    assert!(!grouped.contains("q/1"));
    assert!(grouped.contains("<KeyCount>3</KeyCount>"));

    let page = list("prefix=p%2F&max-keys=2");
    assert!(page.contains("<IsTruncated>true</IsTruncated>"));
    let token = page
        .split("<NextContinuationToken>")
        .nth(1)
        .unwrap()
        .split("</NextContinuationToken>")
        .next()
        .unwrap()
        .to_string();
    let next = list(&format!(
        "prefix=p%2F&max-keys=2&continuation-token={token}"
    ));
    assert!(next.contains("<Key>p/b/1</Key>") && next.contains("<Key>p/c</Key>"));
    assert!(next.contains("<IsTruncated>false</IsTruncated>"));
    assert!(!next.contains("<NextContinuationToken>"));
    let ops: Vec<Op> = fake.log().iter().map(|l| l.op).collect();
    assert_eq!(ops.iter().filter(|&&op| op == Op::List).count(), 3);
}

#[test]
fn delete_many_removes_every_listed_key() {
    let fake = start();
    for key in ["a", "b", "c"] {
        put(&fake, key, b"x").unwrap();
    }
    let body = "<Delete><Object><Key>a</Key></Object><Object><Key>c</Key></Object><Object><Key>missing</Key></Object></Delete>";
    let reply = got(agent()
        .post(format!("{}/test?delete&{CREDENTIAL}", fake.endpoint()))
        .send(body.as_bytes()))
    .unwrap();
    assert_eq!(reply.status, 200);
    assert!(text(&reply).contains("<Deleted><Key>a</Key></Deleted>"));
    let keys: Vec<String> = fake.objects().into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, ["b"]);
    assert_eq!(fake.log().last().unwrap().op, Op::DeleteMany);
}

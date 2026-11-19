//! 端到端离线测试：本地模拟禁漫 API（真实 AES 响应体），验证
//! 请求重试 → 响应解密 → 实体解析 → 图片下载 → PDF 生成 → 中间图片清理 的完整链路。

use std::collections::HashMap;
use std::sync::Arc;

use jmcomic_dl::config::{secret, ClientKind, ImageFormat, Kind};
use jmcomic_dl::crypto;
use jmcomic_dl::download::{self, JobSpec};
use jmcomic_dl::events::{LogSink, Progress, StopFlag};
use jmcomic_dl::pdf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 生成测试用 JPEG。
fn sample_jpeg(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8])
    });
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    encoder
        .encode(img.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .expect("encode jpeg");
    out
}

fn read_headers(request: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for line in request.lines().skip(1) {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    headers
}

fn encrypted_body(json: &str, ts: i64) -> Vec<u8> {
    let encrypted = crypto::encode_resp_data(json, ts, secret::APP_DATA_SECRET).expect("encrypt");
    format!(r#"{{"code":200,"data":"{encrypted}"}}"#).into_bytes()
}

fn query_id(path: &str) -> Option<String> {
    let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key == "id" {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn route(path: &str, headers: &HashMap<String, String>, jpeg: &[u8]) -> (String, String, Vec<u8>) {
    let ts = headers
        .get("tokenparam")
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);

    // 只有车号 123 存在于模拟服务器
    let known_id = query_id(path).as_deref() == Some("123");

    if path.starts_with("/setting") {
        let body = encrypted_body(r#"{"jm3_version":"2.0.28","status":"ok"}"#, ts);
        return ("200 OK".to_string(), "application/json".to_string(), body);
    }

    if path.starts_with("/album") {
        if !known_id {
            return (
                "404 Not Found".to_string(),
                "text/plain".to_string(),
                b"album missing".to_vec(),
            );
        }
        let body = encrypted_body(
            r#"{
                "id": 123,
                "name": "测试本子",
                "author": ["测试作者"],
                "tags": ["全彩"],
                "description": null,
                "series": [{"id": "123", "name": "测试本子", "sort": "2"}]
            }"#,
            ts,
        );
        return ("200 OK".to_string(), "application/json".to_string(), body);
    }

    if path.starts_with("/chapter_view_template") {
        // 该接口返回 HTML 文本而非加密 JSON
        return (
            "200 OK".to_string(),
            "text/html".to_string(),
            b"var scramble_id = 220980;".to_vec(),
        );
    }

    if path.starts_with("/chapter") {
        if !known_id {
            return (
                "404 Not Found".to_string(),
                "text/plain".to_string(),
                b"photo missing".to_vec(),
            );
        }
        let body = encrypted_body(
            r#"{
                "id": 123,
                "name": "测试本子",
                "images": ["00001.jpg", "00002.jpg"],
                "series_id": "0",
                "series": [{"id": "123", "name": "测试本子", "sort": "2"}],
                "tags": "全彩"
            }"#,
            ts,
        );
        return ("200 OK".to_string(), "application/json".to_string(), body);
    }

    if path.contains("/media/photos/123/") {
        return (
            "200 OK".to_string(),
            "image/jpeg".to_string(),
            jpeg.to_vec(),
        );
    }

    (
        "404 Not Found".to_string(),
        "text/plain".to_string(),
        b"not found".to_vec(),
    )
}

/// 启动本地模拟服务器，返回 `http://127.0.0.1:port`。
async fn spawn_mock(jpeg: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let jpeg = Arc::new(jpeg);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let jpeg = jpeg.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 16 * 1024];
                let mut filled = 0usize;
                loop {
                    if filled >= buffer.len() {
                        break;
                    }
                    match socket.read(&mut buffer[filled..]).await {
                        Ok(0) => return,
                        Ok(read) => {
                            filled += read;
                            if buffer[..filled]
                                .windows(4)
                                .any(|window| window == b"\r\n\r\n")
                            {
                                break;
                            }
                        }
                        Err(_) => return,
                    }
                }

                let request = String::from_utf8_lossy(&buffer[..filled]).to_string();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let headers = read_headers(&request);
                let (status, content_type, body) = route(&path, &headers, &jpeg);

                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.flush().await;
            });
        }
    });

    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_album_and_exports_pdf_offline() {
    let jpeg = sample_jpeg(40, 60);
    let base = spawn_mock(jpeg.clone()).await;

    let output = std::env::temp_dir().join(format!("jm_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output);

    let spec = JobSpec {
        kind: Kind::Album,
        ids: vec!["123".to_string()],
        output_dir: output.clone(),
        client: ClientKind::Api,
        image_format: ImageFormat::Original,
        image_concurrency: 4,
        photo_concurrency: 2,
        album_concurrency: 1,
        proxy: "none".to_string(),
        retry: 1,
        decode_images: true,
        download_cache: true,
        domains: Some(vec![base.clone()]),
        image_domains: Some(vec![base.clone()]),
    };

    let sink: LogSink = Arc::new(|line: String| println!("{line}"));
    let summary = download::run(spec, sink, StopFlag::new(), Progress::new()).await;

    assert!(
        summary.failures.is_empty(),
        "不应有失败项: {:?}",
        summary.failures
    );
    assert_eq!(summary.albums, 1);
    assert_eq!(summary.photos, 1);
    assert_eq!(summary.images, 2);
    assert_eq!(summary.pdfs, 1);

    let pdf_path = output.join("PDF").join("测试本子.pdf");
    assert!(pdf_path.is_file(), "PDF 应存在: {}", pdf_path.display());

    let bytes = std::fs::read(&pdf_path).unwrap();
    assert_eq!(pdf::count_pages(&bytes), 2);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/DCTDecode"), "JPEG 应直接嵌入而非重新编码");

    // 中间图片与章节目录应被清理，PDF 保留
    assert!(!output.join("测试本子").exists(), "中间图片目录应被清理");

    let _ = std::fs::remove_dir_all(&output);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retries_and_reports_failure_when_album_missing() {
    let jpeg = sample_jpeg(8, 8);
    let base = spawn_mock(jpeg).await;

    let output = std::env::temp_dir().join(format!("jm_e2e_fail_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output);

    let spec = JobSpec {
        kind: Kind::Photo,
        // 该车号在模拟服务器上返回 404，应被记录为失败而不是 panic
        ids: vec!["999".to_string()],
        output_dir: output.clone(),
        client: ClientKind::Api,
        image_format: ImageFormat::Original,
        image_concurrency: 2,
        photo_concurrency: 1,
        album_concurrency: 1,
        proxy: "none".to_string(),
        retry: 0,
        decode_images: true,
        download_cache: true,
        domains: Some(vec![base.clone()]),
        image_domains: Some(vec![base]),
    };

    let sink: LogSink = Arc::new(|line: String| println!("{line}"));
    let summary = download::run(spec, sink, StopFlag::new(), Progress::new()).await;

    assert_eq!(summary.photos, 0);
    assert!(!summary.failures.is_empty(), "应记录失败项");
    assert!(
        summary.failures[0].error.contains("404") || summary.failures[0].error.contains("请求")
    );

    let _ = std::fs::remove_dir_all(&output);
}

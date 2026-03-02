use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload, rand_core::RngCore};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const OPENAI_RESPONSES_HISTORY_FILE_NAME: &str = "openai_responses_history.jsonl";
const OPENAI_RESPONSES_HISTORY_KEY_FILE_NAME: &str = "openai_responses_history.key";
const OPENAI_RESPONSES_HISTORY_CODEC_ENV: &str = "OMNE_OPENAI_RESPONSES_HISTORY_CODEC";
const OPENAI_RESPONSES_HISTORY_KEY_B64_ENV: &str = "OMNE_OPENAI_RESPONSES_HISTORY_KEY_B64";
const OPENAI_RESPONSES_HISTORY_KEY_ENV: &str = "OMNE_OPENAI_RESPONSES_HISTORY_KEY";
const OPENAI_RESPONSES_HISTORY_SEALED_VERSION: u32 = 1;
const OPENAI_RESPONSES_HISTORY_KEY_LEN: usize = 32;
const OPENAI_RESPONSES_HISTORY_NONCE_LEN: usize = 12;
const OPENAI_RESPONSES_HISTORY_AAD_DOMAIN: &str = "omne.openai.responses.history.v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAiResponsesHistoryStoredRecord {
    Sealed {
        version: u32,
        nonce: String,
        ciphertext: String,
    },
    Item {
        item: serde_json::Value,
    },
    Compacted {
        replacement_history: Vec<serde_json::Value>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAiResponsesHistoryLogicalRecord {
    Item { item: serde_json::Value },
    Compacted {
        replacement_history: Vec<serde_json::Value>,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAiResponsesHistoryLogicalRecordRef<'a> {
    Item {
        item: &'a serde_json::Value,
    },
    Compacted {
        replacement_history: &'a [serde_json::Value],
    },
}

impl<'a> OpenAiResponsesHistoryLogicalRecordRef<'a> {
    fn to_owned(&self) -> OpenAiResponsesHistoryLogicalRecord {
        match self {
            Self::Item { item } => OpenAiResponsesHistoryLogicalRecord::Item { item: (*item).clone() },
            Self::Compacted {
                replacement_history,
            } => OpenAiResponsesHistoryLogicalRecord::Compacted {
                replacement_history: replacement_history.to_vec(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAiResponsesHistoryCodecMode {
    Plaintext,
    Encrypted,
}

fn openai_responses_history_codec_mode() -> OpenAiResponsesHistoryCodecMode {
    match std::env::var(OPENAI_RESPONSES_HISTORY_CODEC_ENV)
        .unwrap_or_else(|_| "plaintext".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "plaintext" | "plain" | "off" => OpenAiResponsesHistoryCodecMode::Plaintext,
        "encrypted" | "encrypt" => OpenAiResponsesHistoryCodecMode::Encrypted,
        value => {
            tracing::warn!(
                env = OPENAI_RESPONSES_HISTORY_CODEC_ENV,
                value,
                "unknown openai history codec; defaulting to plaintext"
            );
            OpenAiResponsesHistoryCodecMode::Plaintext
        }
    }
}

#[derive(Debug, Clone)]
struct OpenAiResponsesHistoryCodec {
    mode: OpenAiResponsesHistoryCodecMode,
    key: Option<[u8; OPENAI_RESPONSES_HISTORY_KEY_LEN]>,
}

impl OpenAiResponsesHistoryCodec {
    async fn for_thread(
        thread_store: &omne_core::ThreadStore,
        thread_id: omne_protocol::ThreadId,
    ) -> anyhow::Result<Self> {
        let mode = openai_responses_history_codec_mode();
        if mode == OpenAiResponsesHistoryCodecMode::Plaintext {
            return Ok(Self { mode, key: None });
        }
        let key = load_openai_responses_history_key(thread_store, thread_id).await?;
        Ok(Self {
            mode,
            key: Some(key),
        })
    }

    fn is_encrypted(&self) -> bool {
        self.mode == OpenAiResponsesHistoryCodecMode::Encrypted
    }

    fn encode_record_ref(
        &self,
        thread_id: omne_protocol::ThreadId,
        record: &OpenAiResponsesHistoryLogicalRecordRef<'_>,
    ) -> anyhow::Result<OpenAiResponsesHistoryStoredRecord> {
        if !self.is_encrypted() {
            return Ok(match record {
                OpenAiResponsesHistoryLogicalRecordRef::Item { item } => {
                    OpenAiResponsesHistoryStoredRecord::Item { item: (*item).clone() }
                }
                OpenAiResponsesHistoryLogicalRecordRef::Compacted {
                    replacement_history,
                } => OpenAiResponsesHistoryStoredRecord::Compacted {
                    replacement_history: replacement_history.to_vec(),
                },
            });
        }

        let logical = record.to_owned();
        let plaintext = serde_json::to_vec(&logical).context("serialize openai history payload")?;
        let key = self
            .key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing openai history key"))?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(key));

        let mut nonce_bytes = [0u8; OPENAI_RESPONSES_HISTORY_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let aad = openai_responses_history_aad(thread_id);
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: &plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("encrypt openai history payload failed"))?;

        Ok(OpenAiResponsesHistoryStoredRecord::Sealed {
            version: OPENAI_RESPONSES_HISTORY_SEALED_VERSION,
            nonce: base64::engine::general_purpose::STANDARD_NO_PAD.encode(nonce_bytes),
            ciphertext: base64::engine::general_purpose::STANDARD_NO_PAD.encode(ciphertext),
        })
    }

    fn decode_record(
        &self,
        thread_id: omne_protocol::ThreadId,
        record: OpenAiResponsesHistoryStoredRecord,
    ) -> anyhow::Result<(OpenAiResponsesHistoryLogicalRecord, bool)> {
        match record {
            OpenAiResponsesHistoryStoredRecord::Item { item } => {
                Ok((OpenAiResponsesHistoryLogicalRecord::Item { item }, true))
            }
            OpenAiResponsesHistoryStoredRecord::Compacted {
                replacement_history,
            } => Ok((
                OpenAiResponsesHistoryLogicalRecord::Compacted {
                    replacement_history,
                },
                true,
            )),
            OpenAiResponsesHistoryStoredRecord::Sealed {
                version,
                nonce,
                ciphertext,
            } => {
                if version != OPENAI_RESPONSES_HISTORY_SEALED_VERSION {
                    anyhow::bail!(
                        "unsupported openai history sealed version: {} (expected {})",
                        version,
                        OPENAI_RESPONSES_HISTORY_SEALED_VERSION
                    );
                }

                let key = self
                    .key
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("missing openai history key"))?;
                let nonce_bytes = decode_base64_bytes(&nonce, "sealed nonce")?;
                if nonce_bytes.len() != OPENAI_RESPONSES_HISTORY_NONCE_LEN {
                    anyhow::bail!(
                        "invalid openai history nonce length: {} (expected {})",
                        nonce_bytes.len(),
                        OPENAI_RESPONSES_HISTORY_NONCE_LEN
                    );
                }
                let mut nonce_raw = [0u8; OPENAI_RESPONSES_HISTORY_NONCE_LEN];
                nonce_raw.copy_from_slice(&nonce_bytes);

                let ciphertext = decode_base64_bytes(&ciphertext, "sealed ciphertext")?;
                let aad = openai_responses_history_aad(thread_id);
                let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
                let plaintext = cipher
                    .decrypt(
                        Nonce::from_slice(&nonce_raw),
                        Payload {
                            msg: ciphertext.as_ref(),
                            aad: aad.as_bytes(),
                        },
                    )
                    .map_err(|_| anyhow::anyhow!("decrypt openai history payload failed"))?;

                let logical = serde_json::from_slice::<OpenAiResponsesHistoryLogicalRecord>(
                    &plaintext,
                )
                .context("parse decrypted openai history payload")?;
                Ok((logical, false))
            }
        }
    }
}

fn openai_responses_history_path(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
) -> std::path::PathBuf {
    thread_store
        .thread_dir(thread_id)
        .join(OPENAI_RESPONSES_HISTORY_FILE_NAME)
}

fn openai_responses_history_aad(thread_id: omne_protocol::ThreadId) -> String {
    format!("{OPENAI_RESPONSES_HISTORY_AAD_DOMAIN}:{thread_id}")
}

fn openai_responses_history_key_path(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
) -> std::path::PathBuf {
    let thread_dir = thread_store.thread_dir(thread_id);
    let omne_root = thread_dir
        .parent()
        .and_then(|dir| dir.parent())
        .map(std::path::Path::to_path_buf)
        .unwrap_or(thread_dir);
    omne_root
        .join("keys")
        .join(OPENAI_RESPONSES_HISTORY_KEY_FILE_NAME)
}

async fn ensure_openai_responses_history_file_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        if let Err(err) = tokio::fs::set_permissions(path, perm).await {
            tracing::debug!(
                path = %path.display(),
                error = %err,
                "failed to tighten openai history file permissions"
            );
        }
    }
}

async fn ensure_openai_responses_history_key_file_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        if let Err(err) = tokio::fs::set_permissions(path, perm).await {
            tracing::debug!(
                path = %path.display(),
                error = %err,
                "failed to tighten openai history key file permissions"
            );
        }
    }
}

fn decode_base64_bytes(value: &str, field: &str) -> anyhow::Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(value))
        .with_context(|| format!("decode {field} as base64"))
}

fn parse_local_history_secret(bytes: &[u8]) -> anyhow::Result<[u8; OPENAI_RESPONSES_HISTORY_KEY_LEN]> {
    if bytes.len() == OPENAI_RESPONSES_HISTORY_KEY_LEN {
        let mut key = [0u8; OPENAI_RESPONSES_HISTORY_KEY_LEN];
        key.copy_from_slice(bytes);
        return Ok(key);
    }

    let text = String::from_utf8_lossy(bytes).trim().to_string();
    if text.is_empty() {
        anyhow::bail!("openai history key file is empty");
    }
    let decoded = decode_base64_bytes(&text, "openai history key file")?;
    if decoded.len() < OPENAI_RESPONSES_HISTORY_KEY_LEN {
        anyhow::bail!(
            "openai history key file is too short: {} (expected at least {})",
            decoded.len(),
            OPENAI_RESPONSES_HISTORY_KEY_LEN
        );
    }
    let mut key = [0u8; OPENAI_RESPONSES_HISTORY_KEY_LEN];
    key.copy_from_slice(&decoded[..OPENAI_RESPONSES_HISTORY_KEY_LEN]);
    Ok(key)
}

fn parse_env_history_key_material() -> anyhow::Result<Option<Vec<u8>>> {
    if let Ok(value) = std::env::var(OPENAI_RESPONSES_HISTORY_KEY_B64_ENV) {
        let value = value.trim();
        if !value.is_empty() {
            return decode_base64_bytes(value, OPENAI_RESPONSES_HISTORY_KEY_B64_ENV).map(Some);
        }
    }

    if let Ok(value) = std::env::var(OPENAI_RESPONSES_HISTORY_KEY_ENV) {
        if !value.is_empty() {
            return Ok(Some(value.into_bytes()));
        }
    }

    Ok(None)
}

fn derive_openai_responses_history_key(
    thread_id: omne_protocol::ThreadId,
    material: &[u8],
) -> [u8; OPENAI_RESPONSES_HISTORY_KEY_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(OPENAI_RESPONSES_HISTORY_AAD_DOMAIN.as_bytes());
    hasher.update([0u8]);
    hasher.update(thread_id.to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(material);
    let digest = hasher.finalize();
    let mut key = [0u8; OPENAI_RESPONSES_HISTORY_KEY_LEN];
    key.copy_from_slice(&digest[..OPENAI_RESPONSES_HISTORY_KEY_LEN]);
    key
}

async fn load_or_create_local_openai_history_secret(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
) -> anyhow::Result<[u8; OPENAI_RESPONSES_HISTORY_KEY_LEN]> {
    let path = openai_responses_history_key_path(thread_store, thread_id);
    loop {
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let key = parse_local_history_secret(&bytes)
                    .with_context(|| format!("parse {}", path.display()))?;
                ensure_openai_responses_history_key_file_permissions(&path).await;
                return Ok(key);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .with_context(|| format!("mkdir {}", parent.display()))?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = tokio::fs::set_permissions(
                            parent,
                            std::fs::Permissions::from_mode(0o700),
                        )
                        .await;
                    }
                }

                let mut secret = [0u8; OPENAI_RESPONSES_HISTORY_KEY_LEN];
                OsRng.fill_bytes(&mut secret);
                match tokio::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                    .await
                {
                    Ok(mut file) => {
                        file.write_all(&secret).await?;
                        ensure_openai_responses_history_key_file_permissions(&path).await;
                        return Ok(secret);
                    }
                    Err(create_err) if create_err.kind() == std::io::ErrorKind::AlreadyExists => {
                        continue;
                    }
                    Err(create_err) => {
                        return Err(create_err)
                            .with_context(|| format!("create {}", path.display()));
                    }
                }
            }
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        }
    }
}

async fn load_openai_responses_history_key(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
) -> anyhow::Result<[u8; OPENAI_RESPONSES_HISTORY_KEY_LEN]> {
    if let Some(material) = parse_env_history_key_material()? {
        return Ok(derive_openai_responses_history_key(thread_id, &material));
    }
    let local_secret = load_or_create_local_openai_history_secret(thread_store, thread_id).await?;
    Ok(derive_openai_responses_history_key(
        thread_id,
        &local_secret,
    ))
}

async fn append_openai_responses_history_items(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    items: &[serde_json::Value],
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }

    let records = items
        .iter()
        .map(|item| OpenAiResponsesHistoryLogicalRecordRef::Item { item })
        .collect::<Vec<_>>();
    append_openai_responses_history_records_ref(thread_store, thread_id, &records).await
}

async fn append_openai_responses_history_records_ref(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    records: &[OpenAiResponsesHistoryLogicalRecordRef<'_>],
) -> anyhow::Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let codec = OpenAiResponsesHistoryCodec::for_thread(thread_store, thread_id).await?;
    let path = openai_responses_history_path(thread_store, thread_id);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    ensure_openai_responses_history_file_permissions(&path).await;

    for record in records {
        let stored = codec.encode_record_ref(thread_id, record)?;
        let line = serde_json::to_string(&stored).context("serialize openai history record")?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }

    Ok(())
}

async fn append_openai_responses_history_compacted(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    replacement_history: &[serde_json::Value],
) -> anyhow::Result<()> {
    append_openai_responses_history_records_ref(
        thread_store,
        thread_id,
        &[OpenAiResponsesHistoryLogicalRecordRef::Compacted {
            replacement_history,
        }],
    )
    .await
}

async fn rewrite_openai_responses_history_as_compacted(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    replacement_history: &[serde_json::Value],
    codec: &OpenAiResponsesHistoryCodec,
) -> anyhow::Result<()> {
    let stored = codec.encode_record_ref(
        thread_id,
        &OpenAiResponsesHistoryLogicalRecordRef::Compacted {
            replacement_history,
        },
    )?;
    let line = serde_json::to_string(&stored).context("serialize compacted openai history record")?;
    let path = openai_responses_history_path(thread_store, thread_id);
    tokio::fs::write(&path, format!("{line}\n").as_bytes())
        .await
        .with_context(|| format!("write {}", path.display()))?;
    ensure_openai_responses_history_file_permissions(&path).await;
    Ok(())
}

async fn read_openai_responses_history(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let path = openai_responses_history_path(thread_store, thread_id);
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
    };

    let codec = OpenAiResponsesHistoryCodec::for_thread(thread_store, thread_id).await?;
    let mut lines = BufReader::new(file).lines();
    let mut history = Vec::<serde_json::Value>::new();
    let mut idx: usize = 0;
    let mut saw_legacy_records = false;
    while let Some(line) = lines
        .next_line()
        .await
        .with_context(|| format!("read {}", path.display()))?
    {
        idx = idx.saturating_add(1);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record =
            serde_json::from_str::<OpenAiResponsesHistoryStoredRecord>(line).with_context(
            || format!("parse openai history record: {} (line={idx})", path.display()),
        )?;
        let (record, legacy) = codec
            .decode_record(thread_id, record)
            .with_context(|| format!("decode openai history record: {} (line={idx})", path.display()))?;
        saw_legacy_records |= legacy;

        match record {
            OpenAiResponsesHistoryLogicalRecord::Item { item } => history.push(item),
            OpenAiResponsesHistoryLogicalRecord::Compacted {
                replacement_history,
            } => history = replacement_history,
        }
    }

    if codec.is_encrypted() && saw_legacy_records {
        rewrite_openai_responses_history_as_compacted(thread_store, thread_id, &history, &codec)
            .await?;
    }

    Ok(history)
}

async fn compact_openai_responses_history(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    client: &ditto_llm::OpenAI,
    model: &str,
    instructions: &str,
    input: &[serde_json::Value],
) -> anyhow::Result<Vec<serde_json::Value>> {
    let replacement_history = client
        .compact_responses_history_raw(&ditto_llm::providers::openai::OpenAIResponsesCompactionRequest {
            model,
            input,
            instructions,
        })
        .await
        .map_err(anyhow::Error::new)?;

    append_openai_responses_history_compacted(thread_store, thread_id, &replacement_history).await?;

    Ok(replacement_history)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn openai_history_replays_compaction_replacement() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let thread_store = omne_core::ThreadStore::new(omne_core::PmPaths::new(
            dir.path().join(".omne_data"),
        ));

        let handle = thread_store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_openai_responses_history_items(
            &thread_store,
            thread_id,
            &[serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"a"}]})],
        )
        .await?;

        let replacement_history =
            vec![serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"b"}]})];
        append_openai_responses_history_compacted(&thread_store, thread_id, &replacement_history)
            .await?;

        append_openai_responses_history_items(
            &thread_store,
            thread_id,
            &[serde_json::json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"c"}]})],
        )
        .await?;

        let history = read_openai_responses_history(&thread_store, thread_id).await?;
        assert_eq!(
            history,
            vec![
                serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"b"}]}),
                serde_json::json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"c"}]}),
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn openai_history_reads_legacy_plaintext_and_migrates_when_encrypted() -> anyhow::Result<()>
    {
        let dir = tempdir()?;
        let thread_store = omne_core::ThreadStore::new(omne_core::PmPaths::new(
            dir.path().join(".omne_data"),
        ));

        let handle = thread_store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let path = openai_responses_history_path(&thread_store, thread_id);
        let legacy_line = serde_json::to_string(&OpenAiResponsesHistoryStoredRecord::Item {
            item: serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"legacy"}]}),
        })?;
        tokio::fs::write(&path, format!("{legacy_line}\n")).await?;

        let history = read_openai_responses_history(&thread_store, thread_id).await?;
        assert_eq!(
            history,
            vec![serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"legacy"}]})]
        );

        let raw = tokio::fs::read_to_string(&path).await?;
        if openai_responses_history_codec_mode() == OpenAiResponsesHistoryCodecMode::Encrypted {
            assert!(raw.contains("\"type\":\"sealed\""));
            assert!(!raw.contains("\"type\":\"item\""));
        }

        Ok(())
    }
}

//! Runtime provider model builders.
//!
//! These builders are the runtime assembly layer that turns resolved provider/config input
//! into concrete capability adapters. `gateway` consumes them, but does not own them.

use std::sync::Arc;

use super::builder_backends;
use super::builder_protocol::{
    BuilderAssemblyRequest, configured_default_model, default_builder_assembly,
    resolve_builder_assembly, resolve_context_cache_assembly,
};
use super::builtin::builtin_runtime_assembly;
use crate::capabilities::audio::{AudioTranscriptionModel, SpeechModel};
use crate::capabilities::embedding::EmbeddingModel;
use crate::capabilities::file::FileClient;
use crate::capabilities::{BatchClient, ContextCacheModel};
use crate::capabilities::{ImageGenerationModel, ModerationModel, RerankModel};
use crate::config::{Env, ProviderConfig};
use crate::contracts::CapabilityKind;
use crate::foundation::error::Result;
use crate::llm_core::model::LanguageModel;

// RUNTIME-BUILDER-SUPPORT-FRONTDOOR: gateway/application callers ask runtime
// whether a provider/config can assemble a capability adapter instead of
// reaching into runtime_registry singletons directly.
pub fn builtin_runtime_supports_capability(
    provider: &str,
    config: &ProviderConfig,
    model: Option<&str>,
    capability: CapabilityKind,
) -> bool {
    let runtime = builtin_runtime_assembly();
    let provider = provider.trim();
    if provider.is_empty() {
        return false;
    }

    let requested_model = if capability == CapabilityKind::BATCH {
        None
    } else {
        model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| configured_default_model(config))
    };

    runtime
        .registry()
        .provider_supports_capability(provider, config, requested_model, capability)
}

pub fn builtin_runtime_supports_file_builder(provider: &str, config: &ProviderConfig) -> bool {
    let runtime = builtin_runtime_assembly();
    let provider = provider.trim();
    if provider.is_empty() {
        return false;
    }
    runtime
        .registry()
        .provider_supports_file_builder(provider, config)
}

// RUNTIME-BUILDER-FRONTDOOR: public runtime builder entrypoints resolve the
// assembly plan, then delegate provider-specific instantiation to
// `builder_backends`.

pub async fn build_language_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Arc<dyn LanguageModel>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::LLM,
    ))?;
    builder_backends::build_language_model(&plan, _env).await
}

pub async fn build_embedding_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn EmbeddingModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::EMBEDDING,
    ))?;
    builder_backends::build_embedding_model(&plan, _env).await
}

pub async fn build_moderation_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn ModerationModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::MODERATION,
    ))?;
    builder_backends::build_moderation_model(&plan, _env).await
}

pub async fn build_image_generation_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn ImageGenerationModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::IMAGE_GENERATION,
    ))?;
    builder_backends::build_image_generation_model(&plan, _env).await
}

pub async fn build_image_edit_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn crate::capabilities::ImageEditModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::IMAGE_EDIT,
    ))?;
    builder_backends::build_image_edit_model(&plan, _env).await
}

pub async fn build_video_generation_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn crate::capabilities::video::VideoGenerationModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::VIDEO_GENERATION,
    ))?;
    builder_backends::build_video_generation_model(&plan, _env).await
}

pub async fn build_realtime_session_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn crate::capabilities::realtime::RealtimeSessionModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::REALTIME,
    ))?;
    builder_backends::build_realtime_session_model(&plan, _env).await
}

pub async fn build_audio_transcription_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn AudioTranscriptionModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::AUDIO_TRANSCRIPTION,
    ))?;
    builder_backends::build_audio_transcription_model(&plan, _env).await
}

pub async fn build_speech_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn SpeechModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::AUDIO_SPEECH,
    ))?;
    builder_backends::build_speech_model(&plan, _env).await
}

pub async fn build_batch_client(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn BatchClient>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::BATCH,
    ))?;
    builder_backends::build_batch_client(&plan, _env).await
}

pub async fn build_rerank_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn RerankModel>>> {
    let plan = resolve_builder_assembly(BuilderAssemblyRequest::new(
        provider,
        config,
        CapabilityKind::RERANK,
    ))?;
    builder_backends::build_rerank_model(&plan, _env).await
}

pub async fn build_file_client(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn FileClient>>> {
    let plan = match default_builder_assembly(provider, config) {
        Ok(plan) => plan,
        Err(_) => return Ok(None),
    };
    builder_backends::build_file_client(&plan, _env).await
}

pub async fn build_context_cache_model(
    provider: &str,
    config: &ProviderConfig,
    _env: &Env,
) -> Result<Option<Arc<dyn ContextCacheModel>>> {
    let plan = resolve_context_cache_assembly(provider, config)?;
    builder_backends::build_context_cache_model(&plan, _env).await
}

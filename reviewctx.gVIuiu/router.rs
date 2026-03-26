use super::a2a::{handle_a2a_agent_card, handle_a2a_invoke};
#[cfg(feature = "gateway-proxy-cache")]
use super::admin::purge_proxy_cache;
use super::admin::{delete_key, list_keys, upsert_key, upsert_key_with_id};
#[cfg(any(
    feature = "gateway-store-sqlite",
    feature = "gateway-store-postgres",
    feature = "gateway-store-mysql",
    feature = "gateway-store-redis"
))]
use super::admin::{
    export_audit_logs, list_audit_logs, list_budget_ledgers, list_project_budget_ledgers,
    list_tenant_budget_ledgers, list_user_budget_ledgers, reap_reservations,
};
#[cfg(feature = "gateway-routing-advanced")]
use super::admin::{list_backends, reset_backend};
#[cfg(all(
    feature = "gateway-costing",
    any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ),
))]
use super::admin::{
    list_cost_ledgers, list_project_cost_ledgers, list_tenant_cost_ledgers, list_user_cost_ledgers,
};
use super::anthropic::{handle_anthropic_count_tokens, handle_anthropic_messages};
use super::google_genai::{handle_fallback, handle_google_genai};
use super::litellm_keys::litellm_key_router;
use super::mcp::{
    handle_mcp_namespaced_root, handle_mcp_namespaced_subpath, handle_mcp_root, handle_mcp_subpath,
    handle_mcp_tools_call, handle_mcp_tools_list,
};
use super::openai_compat_proxy_path_normalize::handle_openai_compat_proxy_root;
use super::openai_models::handle_openai_models_list;
use super::*;

use axum::Router;
use axum::routing::{any, get, post, put};

fn base_http_router() -> Router<GatewayHttpState> {
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/gateway", post(handle_gateway))
        .route(
            "/a2a/:agent_id/.well-known/agent-card.json",
            get(handle_a2a_agent_card),
        )
        .route("/a2a/:agent_id", post(handle_a2a_invoke))
        .route("/a2a/:agent_id/message/send", post(handle_a2a_invoke))
        .route("/a2a/:agent_id/message/stream", post(handle_a2a_invoke))
        .route("/v1/a2a/:agent_id/message/send", post(handle_a2a_invoke))
        .route("/v1/a2a/:agent_id/message/stream", post(handle_a2a_invoke))
        .route("/mcp/tools/list", any(handle_mcp_tools_list))
        .route("/mcp/tools/call", any(handle_mcp_tools_call))
        .route("/mcp", any(handle_mcp_root))
        .route("/mcp/", any(handle_mcp_root))
        .route("/mcp/*subpath", any(handle_mcp_subpath))
        .route("/:mcp_servers/mcp", any(handle_mcp_namespaced_root))
        .route(
            "/:mcp_servers/mcp/*path",
            any(handle_mcp_namespaced_subpath),
        )
        .route("/chat/completions", any(handle_openai_compat_proxy_root))
        .route("/completions", any(handle_openai_compat_proxy_root))
        .route("/embeddings", any(handle_openai_compat_proxy_root))
        .route("/moderations", any(handle_openai_compat_proxy_root))
        .route("/images/generations", any(handle_openai_compat_proxy_root))
        .route(
            "/audio/transcriptions",
            any(handle_openai_compat_proxy_root),
        )
        .route("/audio/translations", any(handle_openai_compat_proxy_root))
        .route("/audio/speech", any(handle_openai_compat_proxy_root))
        .route("/files", any(handle_openai_compat_proxy_root))
        .route("/files/*path", any(handle_openai_compat_proxy))
        .route("/rerank", any(handle_openai_compat_proxy_root))
        .route("/batches", any(handle_openai_compat_proxy_root))
        .route("/batches/*path", any(handle_openai_compat_proxy))
        .route("/models", get(handle_openai_models_list))
        .route("/models/*path", any(handle_openai_compat_proxy))
        .route("/v1/models", get(handle_openai_models_list))
        .route("/responses", any(handle_openai_compat_proxy_root))
        .route("/responses/compact", any(handle_openai_compat_proxy_root))
        .route("/responses/*path", any(handle_openai_compat_proxy))
        .route("/messages", post(handle_anthropic_messages))
        .route(
            "/messages/count_tokens",
            post(handle_anthropic_count_tokens),
        )
        .route("/v1/messages", post(handle_anthropic_messages))
        .route(
            "/v1/messages/count_tokens",
            post(handle_anthropic_count_tokens),
        )
        .route("/v1beta/models/*path", post(handle_google_genai))
        .route("/v1/*path", any(handle_openai_compat_proxy))
        .fallback(handle_fallback)
}

#[cfg(feature = "gateway-metrics-prometheus")]
fn attach_prometheus_http_routes(router: Router<GatewayHttpState>) -> Router<GatewayHttpState> {
    router.route("/metrics/prometheus", get(metrics_prometheus))
}

#[cfg(not(feature = "gateway-metrics-prometheus"))]
fn attach_prometheus_http_routes(router: Router<GatewayHttpState>) -> Router<GatewayHttpState> {
    router
}

fn attach_admin_http_routes(
    mut router: Router<GatewayHttpState>,
    state: &GatewayHttpState,
) -> Router<GatewayHttpState> {
    if state.admin.admin_token.is_some() || state.admin.admin_read_token.is_some() {
        router = router
            .route("/admin/config/version", get(get_config_version))
            .route("/admin/config/versions", get(list_config_versions))
            .route("/admin/config/export", get(export_config))
            .route("/admin/config/validate", post(validate_config_payload))
            .route("/admin/config/diff", get(diff_config_versions))
            .route(
                "/admin/config/versions/:version_id",
                get(get_config_version_by_id),
            );
    }
    if state.admin.admin_token.is_some() {
        router = router
            .route("/admin/config/router", put(upsert_config_router))
            .route("/admin/config/rollback", post(rollback_config_version));
    }

    let mut keys_router = get(list_keys);
    if state.has_admin_write_tokens() {
        keys_router = keys_router.post(upsert_key);
    }
    router = router.route("/admin/keys", keys_router);

    if state.has_admin_write_tokens() {
        router = router.route(
            "/admin/keys/:id",
            put(upsert_key_with_id).delete(delete_key),
        );
    }

    #[cfg(feature = "gateway-proxy-cache")]
    if state.proxy.cache.is_some() && state.admin.admin_token.is_some() {
        router = router.route("/admin/proxy_cache/purge", post(purge_proxy_cache));
    }

    #[cfg(feature = "gateway-routing-advanced")]
    {
        router = router.route("/admin/backends", get(list_backends));
        if state.admin.admin_token.is_some() {
            router = router.route("/admin/backends/:name/reset", post(reset_backend));
        }
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        router = router
            .route("/admin/audit", get(list_audit_logs))
            .route("/admin/audit/export", get(export_audit_logs));
    }

    #[cfg(any(
        feature = "gateway-store-sqlite",
        feature = "gateway-store-postgres",
        feature = "gateway-store-mysql",
        feature = "gateway-store-redis"
    ))]
    {
        router = router
            .route("/admin/budgets", get(list_budget_ledgers))
            .route("/admin/budgets/tenants", get(list_tenant_budget_ledgers))
            .route("/admin/budgets/projects", get(list_project_budget_ledgers))
            .route("/admin/budgets/users", get(list_user_budget_ledgers));

        #[cfg(feature = "gateway-costing")]
        {
            router = router
                .route("/admin/costs", get(list_cost_ledgers))
                .route("/admin/costs/tenants", get(list_tenant_cost_ledgers))
                .route("/admin/costs/projects", get(list_project_cost_ledgers))
                .route("/admin/costs/users", get(list_user_cost_ledgers));
        }

        if state.admin.admin_token.is_some() {
            router = router.route("/admin/reservations/reap", post(reap_reservations));
        }
    }

    router.merge(litellm_key_router())
}

#[cfg(feature = "gateway-routing-advanced")]
fn start_gateway_background_tasks(state: &mut GatewayHttpState) {
    state.proxy.health_check_task = start_proxy_health_checks(state);
}

#[cfg(not(feature = "gateway-routing-advanced"))]
fn start_gateway_background_tasks(_state: &mut GatewayHttpState) {}

pub fn router(state: GatewayHttpState) -> Router {
    let mut state = state;
    let mut router = attach_prometheus_http_routes(base_http_router());

    if state.has_any_admin_tokens() {
        router = attach_admin_http_routes(router, &state);
    }

    start_gateway_background_tasks(&mut state);
    router.with_state(state)
}

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    proxy_service::{
        GenerateApiTokenOpenApi, GetAllApiTokensOpenApi, GetAllSubscriptionsOpenApi,
        GetAllTasksOpenApi, HealthOpenApi, LoginOpenApi, RegisterOpenApi, RevokeApiTokenOpenApi,
        GENERATE_API_TOKEN_PATH, GET_ALL_API_TOKENS_PATH, GET_STACKS_PATH, HEALTH_PATH, LOGIN_PATH,
        REGISTER_PATH, REVOKE_API_TOKEN_PATH, SUBSCRIPTIONS_PATH, TASKS_PATH,
    },
    GetCurrentStacksOpenApi,
};

pub fn openapi_router() -> Router {
    #[derive(OpenApi)]
    #[openapi(
        nest(
            (path = HEALTH_PATH, api = HealthOpenApi, tags = ["Health"]),
            (path = GENERATE_API_TOKEN_PATH, api = GenerateApiTokenOpenApi, tags = ["Auth"]),
            (path = REVOKE_API_TOKEN_PATH, api = RevokeApiTokenOpenApi, tags = ["Auth"]),
            (path = REGISTER_PATH, api = RegisterOpenApi, tags = ["Auth"]),
            (path = LOGIN_PATH, api = LoginOpenApi, tags = ["Auth"]),
            (path = GET_ALL_API_TOKENS_PATH, api = GetAllApiTokensOpenApi, tags = ["Auth"]),
            (path = GET_STACKS_PATH, api = GetCurrentStacksOpenApi, tags = ["Stacks"]),
            (path = TASKS_PATH, api = GetAllTasksOpenApi, tags = ["Tasks"]),
            (path = SUBSCRIPTIONS_PATH, api = GetAllSubscriptionsOpenApi, tags = ["Subscriptions"]),
        ),
        tags(
            (name = "health", description = "Health check endpoints"),
            (name = "generate_api_token", description = "Authentication and API token management"),
            (name = "revoke_api_token", description = "Authentication and API token management"),
            (name = "register", description = "Authentication and API token management"),
            (name = "login", description = "Authentication and API token management"),
            (name = "get_all_api_tokens", description = "Authentication and API token management"),
            (name = "get_stacks", description = "Stack operations"),
            (name = "get_tasks", description = "Task management"),
            (name = "get_subscriptions", description = "Node subscription management"),
        ),
        servers(
            (url = "http://localhost:3005", description = "Local server"),
        )
    )]
    struct ApiDoc;

    // Generate the OpenAPI spec and write it to a file in debug mode
    #[cfg(debug_assertions)]
    {
        use std::fs;
        use std::path::Path;

        let spec =
            serde_yaml::to_string(&ApiDoc::openapi()).expect("Failed to serialize OpenAPI spec");

        let docs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
        fs::create_dir_all(&docs_dir).expect("Failed to create docs directory");

        let spec_path = docs_dir.join("openapi.yml");
        fs::write(&spec_path, spec).expect("Failed to write OpenAPI spec to file");

        println!("OpenAPI spec written to: {:?}", spec_path);
    }

    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
}

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::server::chat_completions::{ChatCompletionsOpenApi, CHAT_COMPLETIONS_PATH};
use crate::server::http_server::{
    HealthOpenApi, ModelsOpenApi, NodePublicAddressRegistrationOpenApi, HEALTH_PATH, MODELS_PATH,
    NODE_PUBLIC_ADDRESS_REGISTRATION_PATH,
};

pub fn openapi_routes() -> Router {
    #[derive(OpenApi)]
    #[openapi(
        //modifiers(&SecurityAddon),
        nest(
            (path = HEALTH_PATH, api = HealthOpenApi),
            (path = MODELS_PATH, api = ModelsOpenApi),
            (path = NODE_PUBLIC_ADDRESS_REGISTRATION_PATH, api = NodePublicAddressRegistrationOpenApi),
            (path = CHAT_COMPLETIONS_PATH, api = ChatCompletionsOpenApi),
        ),
        tags(
            (name = "health", description = "Health check"),
            (name = "chat", description = "Chat completions"),
            (name = "models", description = "Models"),
            (name = "node-public-address-registration", description = "Node public address registration"),
            (name = "chat-completions", description = "OpenAI's API chat completions v1 endpoint")
        ),
        servers(
            (url = "http://localhost:8080"),
        )
    )]
    struct ApiDoc;

    // struct SecurityAddon;
    // impl Modify for SecurityAddon {
    //     fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
    //         if let Some(components) = openapi.components.as_mut() {
    //             components.add_security_scheme(
    //                 "bearerAuth",
    //                 SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
    //             )
    //         }
    //     }
    // }

    // Generate the OpenAPI spec and write it to a file
    #[cfg(debug_assertions)]
    {
        use std::fs;
        use std::path::Path;

        // Generate OpenAPI spec
        let spec =
            serde_yaml::to_string(&ApiDoc::openapi()).expect("Failed to serialize OpenAPI spec");

        // Ensure the docs directory exists
        let docs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
        fs::create_dir_all(&docs_dir).expect("Failed to create docs directory");

        // Write the spec to the file
        let spec_path = docs_dir.join("openapi.yml");
        fs::write(&spec_path, spec).expect("Failed to write OpenAPI spec to file");

        println!("OpenAPI spec written to: {:?}", spec_path);
    }

    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
}

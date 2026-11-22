use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};

use crate::{
    models::{
        ClaimableRewardEntry, ClaimableRewardsQueue, ClaimableRewardsRequest,
        ClaimableRewardsSimulation, ClaimReconciliation, ClaimReconciliationStatus,
        PortfolioActivityExportQuery, PortfolioActivityExportRow, PortfolioExposure,
        PortfolioExposureRow,
    },
    routes::{
        activity, claims, portfolio,
    },
};

/// Adds a bearer token security scheme to the generated OpenAPI document.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("Authorization"))),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Portfolio API",
        description = "Portfolio exposure, claimable rewards, claim reconciliation, and activity export endpoints.",
        version = "1.0.0"
    ),
    paths(
        portfolio::get_portfolio_exposure,
        claims::list_claimable_rewards,
        claims::simulate_claimable_rewards,
        claims::get_claim_reconciliation,
        activity::export_portfolio_activity,
    ),
    components(
        schemas(
            PortfolioExposure,
            PortfolioExposureRow,
            ClaimableRewardsRequest,
            ClaimableRewardsQueue,
            ClaimableRewardEntry,
            ClaimableRewardsSimulation,
            ClaimReconciliation,
            ClaimReconciliationStatus,
            PortfolioActivityExportQuery,
            PortfolioActivityExportRow,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Portfolio", description = "Portfolio exposure and activity endpoints"),
        (name = "Claims", description = "Claimable rewards and claim reconciliation endpoints"),
    )
)]
pub struct ApiDoc;
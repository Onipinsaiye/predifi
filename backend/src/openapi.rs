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
        activity, claims, content_reports, editorial_collections, feedback, participation_stats,
        portfolio,
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
        description = "Portfolio exposure, claimable rewards, claim reconciliation, activity export, editorial collections, feedback, content reports, and participation statistics endpoints.",
        version = "1.0.0"
    ),
    paths(
        portfolio::get_portfolio_exposure,
        claims::list_claimable_rewards,
        claims::simulate_claimable_rewards,
        claims::get_claim_reconciliation,
        activity::export_portfolio_activity,
        editorial_collections::list_editorial_collections,
        editorial_collections::get_editorial_collection,
        feedback::submit_feedback,
        content_reports::submit_content_report,
        content_reports::get_content_report_status,
        content_reports::submit_content_report_appeal,
        participation_stats::get_participation_statistics,
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
            editorial_collections::EditorialCollection,
            editorial_collections::EditorialCollectionRevision,
            editorial_collections::EditorialCollectionEntry,
            editorial_collections::EditorialCollectionListQuery,
            feedback::FeedbackSubmission,
            feedback::FeedbackCategory,
            feedback::FeedbackDiagnostics,
            feedback::FeedbackReceipt,
            content_reports::ContentReportSubmission,
            content_reports::ContentReportReason,
            content_reports::ContentReportStatus,
            content_reports::ContentReportStatusResponse,
            content_reports::ContentReportAppealSubmission,
            content_reports::ContentReportAppeal,
            participation_stats::ParticipationStatistics,
            participation_stats::ParticipationStatisticsQuery,
            participation_stats::ParticipationAggregate,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Portfolio", description = "Portfolio exposure and activity endpoints"),
        (name = "Claims", description = "Claimable rewards and claim reconciliation endpoints"),
        (name = "Editorial Collections", description = "Versioned, editorially ordered market collections"),
        (name = "Feedback", description = "Opt-in structured product feedback submissions"),
        (name = "Content Reports", description = "Content report and appeal workflow endpoints"),
        (name = "Participation Statistics", description = "Opt-in thresholded aggregate participation statistics"),
    )
)]
pub struct ApiDoc;
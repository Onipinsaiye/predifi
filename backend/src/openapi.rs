use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};

use crate::{
    models::{
        ClaimableRewardEntry, ClaimableRewardsQueue, ClaimableRewardsRequest,
        ClaimableRewardsSimulation, ClaimReconciliation, ClaimReconciliationStatus,
        CreatorProfile, CreatorProfileCreateRequest, CreatorProfileUpdateRequest,
        CreatorProfileVisibility, DiscussionComment, DiscussionCommentCreateRequest,
        DiscussionCommentRevision, DiscussionCommentUpdateRequest, DiscussionModerationState,
        ModerationAppeal, ModerationAppealRequest, ModerationReport, ModerationReportRequest,
        ModerationReview, ModerationReviewRequest, ModerationTransition,
        PortfolioActivityExportQuery, PortfolioActivityExportRow, PortfolioExposure,
        PortfolioExposureRow,
    },
    routes::{
        activity, claims, creators, discussions, moderation, portfolio,
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
        creators::create_creator_profile,
        creators::update_creator_profile,
        creators::get_creator_profile,
        discussions::create_discussion_comment,
        discussions::update_discussion_comment,
        discussions::list_discussion_comments,
        moderation::submit_moderation_report,
        moderation::submit_moderation_review,
        moderation::submit_moderation_appeal,
        moderation::list_moderation_transitions,
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
            CreatorProfile,
            CreatorProfileCreateRequest,
            CreatorProfileUpdateRequest,
            CreatorProfileVisibility,
            DiscussionComment,
            DiscussionCommentCreateRequest,
            DiscussionCommentUpdateRequest,
            DiscussionCommentRevision,
            DiscussionModerationState,
            ModerationReport,
            ModerationReportRequest,
            ModerationReview,
            ModerationReviewRequest,
            ModerationAppeal,
            ModerationAppealRequest,
            ModerationTransition,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Portfolio", description = "Portfolio exposure and activity endpoints"),
        (name = "Claims", description = "Claimable rewards and claim reconciliation endpoints"),
        (name = "Creators", description = "Creator profile endpoints with wallet ownership proofs"),
        (name = "Discussions", description = "Versioned market discussion endpoints"),
        (name = "Moderation", description = "Auditable content moderation transitions"),
    )
)]
pub struct ApiDoc;
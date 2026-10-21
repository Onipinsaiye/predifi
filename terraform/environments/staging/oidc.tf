# GitHub Actions OIDC Integration for AWS Deployment

variable "github_repository" {
  type        = string
  description = "Target GitHub repository in org/repo format authorized to assume the deploy role"
  default     = "Daluworks/predifi"
}

variable "github_deploy_branch" {
  type        = string
  description = "Allowed git branch for staging deployment role assumption"
  default     = "main"
}

variable "role_max_session_duration_seconds" {
  type        = number
  description = "Maximum session duration in seconds for assumed role (15 to 60 minutes)"
  default     = 900 # 15 minutes
}

# The GitHub Actions OIDC OpenID Connect Provider
# Thumbprint for token.actions.githubusercontent.com
data "aws_iam_openid_connect_provider" "github_actions" {
  count = 0 # Can be toggled if provider already exists in AWS account
  url   = "https://token.actions.githubusercontent.com"
}

# In accounts where the provider does not yet exist, create it:
resource "aws_iam_openid_connect_provider" "github_actions" {
  url             = "https://token.actions.githubusercontent.com"
  client_id_list  = ["sts.amazonaws.com"]
  thumbprint_list = ["6938fd4d98bab03faadb97b34396831e3780aea1", "1c58a3a8518e8759bf075b76b750d4f2df264fcd"]
}

# Trust policy restricting role assumption:
# - Principal: GitHub OIDC provider
# - aud: sts.amazonaws.com
# - sub: repo:Daluworks/predifi:ref:refs/heads/main (only authorized repo and branch, never pull_request)
data "aws_iam_policy_document" "github_actions_oidc_assume_role" {
  statement {
    sid     = "GitHubActionsOIDCAssumeRole"
    effect  = "Allow"
    actions = ["sts:AssumeRoleWithWebIdentity"]

    principals {
      type        = "Federated"
      identifiers = [aws_iam_openid_connect_provider.github_actions.arn]
    }

    condition {
      test     = "StringEquals"
      variable = "token.actions.githubusercontent.com:aud"
      values   = ["sts.amazonaws.com"]
    }

    condition {
      test     = "StringEquals"
      variable = "token.actions.githubusercontent.com:sub"
      values   = ["repo:${var.github_repository}:ref:refs/heads/${var.github_deploy_branch}"]
    }

    condition {
      test     = "StringEquals"
      variable = "token.actions.githubusercontent.com:repository"
      values   = [var.github_repository]
    }
  }
}

# Narrowly scoped IAM role for staging CI deployment
resource "aws_iam_role" "ci_staging_deployer" {
  name                 = "${local.name_prefix}-ci-deployer"
  description          = "Narrowly scoped IAM role assumed via GitHub OIDC for staging deployments"
  assume_role_policy   = data.aws_iam_policy_document.github_actions_oidc_assume_role.json
  max_session_duration = var.role_max_session_duration_seconds

  tags = {
    Name        = "${local.name_prefix}-ci-deployer"
    Environment = "staging"
    Role        = "ci-deployer"
    ManagedBy   = "terraform"
  }
}

# Scoped deployment policy: ECS/ASG rolling update, ECR push, and CloudWatch audit metrics
data "aws_iam_policy_document" "ci_staging_deployer_permissions" {
  statement {
    sid    = "ECRAuthAndImagePush"
    effect = "Allow"
    actions = [
      "ecr:GetAuthorizationToken",
      "ecr:BatchCheckLayerAvailability",
      "ecr:GetDownloadUrlForLayer",
      "ecr:BatchGetImage",
      "ecr:PutImage",
      "ecr:InitiateLayerUpload",
      "ecr:UploadLayerPart",
      "ecr:CompleteLayerUpload"
    ]
    resources = ["*"]
  }

  statement {
    sid    = "AutoScalingInstanceRefresh"
    effect = "Allow"
    actions = [
      "autoscaling:StartInstanceRefresh",
      "autoscaling:DescribeInstanceRefreshes",
      "autoscaling:DescribeAutoScalingGroups"
    ]
    resources = ["*"]
  }
}

resource "aws_iam_role_policy" "ci_staging_deployer" {
  name   = "${local.name_prefix}-ci-deployer-policy"
  role   = aws_iam_role.ci_staging_deployer.id
  policy = data.aws_iam_policy_document.ci_staging_deployer_permissions.json
}

output "ci_staging_deployer_role_arn" {
  description = "ARN of the IAM role for GitHub Actions staging deployment via OIDC"
  value       = aws_iam_role.ci_staging_deployer.arn
}

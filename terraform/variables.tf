variable "project" {
  type        = string
  description = "Short project name used in resource naming."
  default     = "predifi"
}

variable "environment" {
  type        = string
  description = "Deployment environment (e.g. production)."
  default     = "production"
}

variable "aws_region" {
  type        = string
  description = "AWS region for all resources."
  default     = "us-east-1"
}

variable "vpc_id" {
  type        = string
  description = "Existing VPC ID."
}

variable "public_subnet_ids" {
  type        = list(string)
  description = "Public subnets for the load balancer."
}

variable "private_subnet_ids" {
  type        = list(string)
  description = "Private subnets for compute, databases, and monitoring."
}

variable "ami_id" {
  type        = string
  description = "AMI for application and monitoring instances."
}

variable "key_name" {
  type        = string
  description = "EC2 key pair name."
  default     = null
}

variable "domain_name" {
  type        = string
  description = "Apex domain for the API / app (e.g. api.predifi.app)."
}

variable "route53_zone_id" {
  type        = string
  description = "Route53 hosted zone ID for DNS + ACM validation."
}

variable "ssl_sans" {
  type        = list(string)
  description = "Additional SANs for the ACM certificate."
  default     = []
}

variable "create_www_record" {
  type    = bool
  default = false
}

variable "health_check_path" {
  type    = string
  default = "/health"
}

# ── Compute ──────────────────────────────────────────────────────────────────

variable "compute_instance_type" {
  type    = string
  default = "t3.medium"
}

variable "compute_desired_capacity" {
  type    = number
  default = 2
}

variable "compute_min_size" {
  type    = number
  default = 2
}

variable "compute_max_size" {
  type    = number
  default = 6
}

variable "compute_user_data" {
  type        = string
  description = "Optional cloud-init / user-data for app instances."
  default     = ""
}

# ── PostgreSQL ───────────────────────────────────────────────────────────────

variable "db_name" {
  type    = string
  default = "predifi"
}

variable "db_username" {
  type    = string
  default = "predifi"
}

variable "db_engine_version" {
  type    = string
  default = "16.4"
}

variable "db_instance_class" {
  type    = string
  default = "db.t4g.medium"
}

variable "db_allocated_storage" {
  type    = number
  default = 100
}

variable "db_multi_az" {
  type    = bool
  default = true
}

variable "db_backup_retention_days" {
  type        = number
  description = "Automated PostgreSQL backup and point-in-time recovery retention (1-35 days)."
  default     = 7

  validation {
    condition     = var.db_backup_retention_days >= 1 && var.db_backup_retention_days <= 35
    error_message = "db_backup_retention_days must be between 1 and 35."
  }
}

# ── Redis ────────────────────────────────────────────────────────────────────

variable "redis_node_type" {
  type    = string
  default = "cache.t4g.small"
}

variable "redis_num_nodes" {
  type    = number
  default = 2
}

variable "redis_engine_version" {
  type    = string
  default = "7.1"
}

variable "redis_snapshot_retention_days" {
  type        = number
  description = "ElastiCache automated snapshot retention in days; use 0 only for disposable environments."
  default     = 1

  validation {
    condition     = var.redis_snapshot_retention_days >= 0 && var.redis_snapshot_retention_days <= 35
    error_message = "redis_snapshot_retention_days must be between 0 and 35."
  }
}

# ── Monitoring ───────────────────────────────────────────────────────────────

variable "monitoring_instance_type" {
  type    = string
  default = "t3.small"
}

variable "grafana_admin_user" {
  type    = string
  default = "admin"
}

variable "monitoring_allowed_cidrs" {
  type        = list(string)
  description = "CIDRs allowed to reach Grafana/Prometheus UI (prefer VPN/bastion)."
  default     = []
}

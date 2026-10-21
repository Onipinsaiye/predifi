# Prometheus + Grafana monitoring stack on a dedicated private instance.

variable "name_prefix" {
  type = string
}

variable "vpc_id" {
  type = string
}

variable "private_subnet_ids" {
  type = list(string)
}

variable "instance_type" {
  type = string
}

variable "ami_id" {
  type = string
}

variable "key_name" {
  type    = string
  default = null
}

variable "grafana_admin_user" {
  type    = string
  default = "admin"
}

variable "allowed_cidr_blocks" {
  type        = list(string)
  description = "CIDRs allowed to reach Grafana (3000) and Prometheus (9090)."
  default     = []
}

variable "prometheus_scrape_targets" {
  type        = list(string)
  description = "Hostnames to scrape (e.g. ALB DNS). Configured into prometheus.yml via user-data."
  default     = []
}

resource "random_password" "grafana_admin" {
  length  = 24
  special = false
}

locals {
  scrape_list = join(", ", [for t in var.prometheus_scrape_targets : "\"${t}:8080\""])

  # Single source of truth for alerting: backend/grafana/alerts/*.yml.
  alert_rules = trimspace(file("${path.module}/../../../backend/grafana/alerts/predifi_alerts.yml"))
  slo_rules   = trimspace(file("${path.module}/../../../backend/grafana/alerts/predifi_slo_rules.yml"))

  # Every literal line below is indented by at least four spaces so <<-EOT
  # strips exactly that prefix: the shebang and heredoc delimiters must land
  # in column 0 or cloud-init will not run the script.
  user_data = <<-EOT
    #!/bin/bash
    set -euo pipefail
    if ! command -v docker >/dev/null 2>&1; then
      curl -fsSL https://get.docker.com | sh
      systemctl enable --now docker
    fi

    mkdir -p /opt/predifi-monitoring/prometheus/alerts

    # Rule files are read from the repository so the deployed alerts, owners,
    # and runbook links can never drift from the reviewed source. The quoted
    # heredoc delimiter stops bash from expanding {{ $value }} templates.
    cat >/opt/predifi-monitoring/prometheus/alerts/predifi_alerts.yml <<'PREDIFI_ALERTS'
    ${local.alert_rules}
    PREDIFI_ALERTS

    cat >/opt/predifi-monitoring/prometheus/alerts/predifi_slo_rules.yml <<'PREDIFI_SLO_RULES'
    ${local.slo_rules}
    PREDIFI_SLO_RULES

    cat >/opt/predifi-monitoring/prometheus/prometheus.yml <<EOF
    global:
      scrape_interval: 15s
    rule_files:
      - /etc/prometheus/alerts/*.yml
    scrape_configs:
      - job_name: predifi
        metrics_path: /metrics
        static_configs:
          - targets: [${local.scrape_list}]
    EOF

    docker rm -f prometheus grafana 2>/dev/null || true

    docker run -d --name prometheus --restart=always \
      -p 9090:9090 \
      -v /opt/predifi-monitoring/prometheus/prometheus.yml:/etc/prometheus/prometheus.yml \
      -v /opt/predifi-monitoring/prometheus/alerts:/etc/prometheus/alerts \
      prom/prometheus:v2.54.1

    docker run -d --name grafana --restart=always \
      -p 3000:3000 \
      -e GF_SECURITY_ADMIN_USER=${var.grafana_admin_user} \
      -e GF_SECURITY_ADMIN_PASSWORD=${random_password.grafana_admin.result} \
      grafana/grafana:11.2.0
  EOT
}

resource "aws_security_group" "monitoring" {
  name_prefix = "${var.name_prefix}-mon-"
  description = "PrediFi Prometheus + Grafana"
  vpc_id      = var.vpc_id

  dynamic "ingress" {
    for_each = length(var.allowed_cidr_blocks) > 0 ? [1] : []
    content {
      description = "Grafana UI"
      from_port   = 3000
      to_port     = 3000
      protocol    = "tcp"
      cidr_blocks = var.allowed_cidr_blocks
    }
  }

  dynamic "ingress" {
    for_each = length(var.allowed_cidr_blocks) > 0 ? [1] : []
    content {
      description = "Prometheus UI"
      from_port   = 9090
      to_port     = 9090
      protocol    = "tcp"
      cidr_blocks = var.allowed_cidr_blocks
    }
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "${var.name_prefix}-monitoring-sg"
  }

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_instance" "monitoring" {
  ami                    = var.ami_id
  instance_type          = var.instance_type
  subnet_id              = var.private_subnet_ids[0]
  vpc_security_group_ids = [aws_security_group.monitoring.id]
  key_name               = var.key_name

  # Gzipped so the rule files fit inside EC2's 16 KB user-data limit; cloud-init
  # decompresses it transparently.
  user_data_base64 = base64gzip(local.user_data)

  root_block_device {
    volume_size = 40
    encrypted   = true
  }

  tags = {
    Name = "${var.name_prefix}-monitoring"
  }
}

output "instance_id" {
  value = aws_instance.monitoring.id
}

output "grafana_private_dns" {
  value = aws_instance.monitoring.private_dns
}

output "prometheus_private_dns" {
  value = aws_instance.monitoring.private_dns
}

output "grafana_admin_password" {
  value     = random_password.grafana_admin.result
  sensitive = true
}

output "security_group_id" {
  value = aws_security_group.monitoring.id
}

# Kubernetes Deployment Guide

**Document Version:** 1.0
**Last Updated:** 2025-12-20
**Target Audience:** DevOps Engineers, Site Reliability Engineers

## Overview

This guide provides comprehensive instructions for deploying the Antenna Model Service to Kubernetes. The service can be deployed using either raw Kubernetes manifests or Helm charts (recommended).

**Deployment Options:**
1. **Helm Chart** (Recommended) - Flexible, environment-specific configurations
2. **Raw Kubernetes Manifests** - Simple, direct deployment

## Table of Contents

- [Prerequisites](#prerequisites)
- [Quick Start](#quick-start)
- [Helm Deployment](#helm-deployment)
- [Raw Manifest Deployment](#raw-manifest-deployment)
- [Configuration](#configuration)
- [Scaling](#scaling)
- [Monitoring](#monitoring)
- [Troubleshooting](#troubleshooting)
- [Production Best Practices](#production-best-practices)

---

## Prerequisites

### Required Tools

- **kubectl** (v1.25+)
- **Helm** (v3.10+) - if using Helm deployment
- **Docker** - for building images
- Access to a Kubernetes cluster (v1.25+)
- Docker registry (for storing images)

### Cluster Requirements

- **Minimum Node Resources:** 2 vCPU, 4 GB RAM per node
- **Recommended:** 3+ nodes for high availability
- **Storage:** Persistent volume support (for calibration data, optional)
- **Ingress Controller:** NGINX, Traefik, or similar (if using Ingress)

### Build and Push Docker Image

Before deploying, build and push the Docker image:

```bash
# Build image
./scripts/build-docker.sh --tag v0.1.0

# Tag for registry
docker tag antenna-model:v0.1.0 your-registry.com/antenna-model:v0.1.0

# Push to registry
docker push your-registry.com/antenna-model:v0.1.0
```

---

## Quick Start

### Using Helm (Recommended)

```bash
# Install with default values
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model \
  --create-namespace

# Check status
kubectl get pods -n antenna-model
kubectl get svc -n antenna-model

# Test health endpoint
kubectl port-forward -n antenna-model svc/antenna-model 3000:80
curl http://localhost:3000/health
```

### Using Raw Manifests

```bash
# Create namespace
kubectl create namespace antenna-model

# Apply manifests
kubectl apply -f k8s/configmap.yaml -n antenna-model
kubectl apply -f k8s/deployment.yaml -n antenna-model
kubectl apply -f k8s/service.yaml -n antenna-model
kubectl apply -f k8s/pdb.yaml -n antenna-model

# Check status
kubectl get all -n antenna-model
```

---

## Helm Deployment

### Installation

#### Default Installation

```bash
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model \
  --create-namespace
```

#### Development Environment

```bash
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model-dev \
  --create-namespace \
  --set replicaCount=1 \
  --set resources.limits.cpu=500m \
  --set resources.limits.memory=256Mi \
  --set config.logging.level=debug \
  --set autoscaling.enabled=false
```

#### Staging Environment

```bash
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model-staging \
  --create-namespace \
  --set replicaCount=2 \
  --set config.logging.level=debug \
  --set ingress.enabled=true \
  --set ingress.hosts[0].host=antenna-model-staging.example.com \
  --set ingress.hosts[0].paths[0].path=/ \
  --set ingress.hosts[0].paths[0].pathType=Prefix
```

#### Production Environment

```bash
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model-prod \
  --create-namespace \
  --set replicaCount=3 \
  --set autoscaling.enabled=true \
  --set autoscaling.minReplicas=3 \
  --set autoscaling.maxReplicas=20 \
  --set podDisruptionBudget.minAvailable=2 \
  --set image.repository=your-registry.com/antenna-model \
  --set image.tag=v0.1.0 \
  --set ingress.enabled=true \
  --set ingress.hosts[0].host=antenna-model.example.com \
  --set ingress.hosts[0].paths[0].path=/ \
  --set ingress.hosts[0].paths[0].pathType=Prefix \
  --set persistence.enabled=true \
  --set persistence.size=5Gi
```

#### Using Values File

Create a custom `values-prod.yaml`:

```yaml
replicaCount: 3

image:
  repository: your-registry.com/antenna-model
  tag: v0.1.0

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 20
  targetCPUUtilizationPercentage: 70

podDisruptionBudget:
  enabled: true
  minAvailable: 2

ingress:
  enabled: true
  className: nginx
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
  hosts:
    - host: antenna-model.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: antenna-model-tls
      hosts:
        - antenna-model.example.com

persistence:
  enabled: true
  storageClass: fast-ssd
  size: 5Gi

config:
  logging:
    level: info
    format: json
```

Deploy with values file:

```bash
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model-prod \
  --create-namespace \
  --values values-prod.yaml
```

### Upgrading

```bash
# Upgrade to new version
helm upgrade antenna-model ./helm/antenna-model \
  --namespace antenna-model \
  --set image.tag=v0.2.0

# Upgrade with new values
helm upgrade antenna-model ./helm/antenna-model \
  --namespace antenna-model \
  --values values-prod.yaml

# Rollback to previous version
helm rollback antenna-model -n antenna-model
```

### Uninstalling

```bash
helm uninstall antenna-model -n antenna-model
```

---

## Raw Manifest Deployment

### Step-by-Step Deployment

#### 1. Create Namespace

```bash
kubectl create namespace antenna-model
```

#### 2. Update Image Reference

Edit `k8s/deployment.yaml` to use your registry:

```yaml
spec:
  template:
    spec:
      containers:
      - name: antenna-model
        image: your-registry.com/antenna-model:v0.1.0
```

#### 3. Apply Manifests

```bash
# Apply in order
kubectl apply -f k8s/configmap.yaml -n antenna-model
kubectl apply -f k8s/deployment.yaml -n antenna-model
kubectl apply -f k8s/service.yaml -n antenna-model
kubectl apply -f k8s/pdb.yaml -n antenna-model
```

#### 4. Verify Deployment

```bash
# Check pods
kubectl get pods -n antenna-model

# Check logs
kubectl logs -n antenna-model -l app=antenna-model

# Check service
kubectl get svc -n antenna-model
```

### Updating Configuration

Edit `k8s/configmap.yaml` and reapply:

```bash
kubectl apply -f k8s/configmap.yaml -n antenna-model

# Restart pods to pick up new config
kubectl rollout restart deployment/antenna-model -n antenna-model
```

---

## Configuration

### Environment Variables

Configure via ConfigMap or Helm values:

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Log level (trace, debug, info, warn, error) |
| `SERVICE_HOST` | `0.0.0.0` | Service bind address |
| `SERVICE_PORT` | `3000` | Service port |
| `CONFIG_PATH` | `/app/config/service.yaml` | Config file path |

### Service Configuration

Key configuration options in `service.yaml`:

```yaml
service:
  host: "0.0.0.0"
  port: 3000
  request_timeout_seconds: 30
  max_request_body_size_bytes: 10485760  # 10 MB

logging:
  level: "info"
  format: "json"
  request_logging: true

computation:
  default_integration_mode: "default"  # fast, default, high_accuracy
  parallel_batch_threshold: 5
  max_batch_size: 1000
```

### Calibration Data Management

#### Option 1: ConfigMap (Development)

Small calibration datasets can be stored in ConfigMap (size limit: 1 MB):

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: antenna-model-calibration
data:
  antennas.yaml: |
    antennas:
      - id: antenna_1
        # ...
```

#### Option 2: Persistent Volume (Production)

For production, use PersistentVolume:

```bash
# Enable in Helm
helm install antenna-model ./helm/antenna-model \
  --set persistence.enabled=true \
  --set persistence.size=5Gi \
  --set persistence.storageClass=fast-ssd
```

Then copy calibration data:

```bash
# Find pod
POD=$(kubectl get pod -n antenna-model -l app.kubernetes.io/name=antenna-model -o jsonpath='{.items[0].metadata.name}')

# Copy calibration files
kubectl cp calibration_data/antenna_1.bin antenna-model/$POD:/app/calibration_data/
kubectl cp calibration_data/antennas.yaml antenna-model/$POD:/app/calibration_data/
```

#### Option 3: Object Storage (Recommended for Production)

Mount S3/GCS/Azure Blob Storage using CSI drivers:

```yaml
# Example: AWS EFS CSI
volumes:
  - name: calibration-data
    persistentVolumeClaim:
      claimName: efs-calibration-pvc
```

---

## Scaling

### Horizontal Pod Autoscaling (HPA)

HPA is enabled by default in Helm deployments:

```yaml
autoscaling:
  enabled: true
  minReplicas: 2
  maxReplicas: 10
  targetCPUUtilizationPercentage: 70
  targetMemoryUtilizationPercentage: 80
```

Monitor HPA status:

```bash
kubectl get hpa -n antenna-model
kubectl describe hpa antenna-model -n antenna-model
```

### Manual Scaling

```bash
# Scale to 5 replicas
kubectl scale deployment antenna-model -n antenna-model --replicas=5

# With Helm
helm upgrade antenna-model ./helm/antenna-model \
  --set replicaCount=5 \
  --reuse-values
```

### Vertical Scaling

Adjust resource limits:

```bash
helm upgrade antenna-model ./helm/antenna-model \
  --set resources.limits.cpu=2000m \
  --set resources.limits.memory=1Gi \
  --reuse-values
```

---

## Monitoring

### Health Checks

The service provides three health endpoints:

- **`/health`** - Liveness probe (pod is alive)
- **`/ready`** - Readiness probe (pod can accept traffic)
- **`/status`** - Detailed service status

### Kubernetes Probes

Configured in deployment:

```yaml
livenessProbe:
  httpGet:
    path: /health
    port: http
  initialDelaySeconds: 10
  periodSeconds: 10
  timeoutSeconds: 5
  failureThreshold: 3

readinessProbe:
  httpGet:
    path: /ready
    port: http
  initialDelaySeconds: 5
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 3
```

### Viewing Logs

```bash
# All pods
kubectl logs -n antenna-model -l app=antenna-model --tail=100 -f

# Specific pod
kubectl logs -n antenna-model <pod-name> -f

# Previous container (if crashed)
kubectl logs -n antenna-model <pod-name> --previous
```

### Metrics

Enable Prometheus monitoring:

```bash
helm upgrade antenna-model ./helm/antenna-model \
  --set monitoring.enabled=true \
  --set monitoring.serviceMonitor.enabled=true \
  --reuse-values
```

Metrics endpoint: `http://<service>:3000/metrics`

---

## Troubleshooting

### Common Issues

#### Pods Not Starting

**Check events:**
```bash
kubectl describe pod -n antenna-model <pod-name>
```

**Common causes:**
- Image pull errors → Check image repository and credentials
- Resource limits → Check node capacity
- ConfigMap missing → Verify ConfigMap exists

#### Pods Crash Looping

**Check logs:**
```bash
kubectl logs -n antenna-model <pod-name> --previous
```

**Common causes:**
- Configuration errors → Validate `service.yaml`
- Missing calibration data → Check volume mounts
- Out of memory → Increase memory limits

#### Service Not Accessible

**Check service and endpoints:**
```bash
kubectl get svc -n antenna-model
kubectl get endpoints -n antenna-model
```

**Test connectivity:**
```bash
# From within cluster
kubectl run -it --rm debug --image=curlimages/curl --restart=Never -- \
  curl http://antenna-model.antenna-model.svc.cluster.local/health
```

#### High Memory Usage

**Check actual usage:**
```bash
kubectl top pods -n antenna-model
```

**Solutions:**
- Reduce `max_batch_size` in config
- Increase memory limits
- Enable HPA for auto-scaling

### Debugging Commands

```bash
# Get all resources
kubectl get all -n antenna-model

# Describe deployment
kubectl describe deployment antenna-model -n antenna-model

# Check resource usage
kubectl top pods -n antenna-model

# Execute shell in pod
kubectl exec -it -n antenna-model <pod-name> -- /bin/sh

# Port forward for local testing
kubectl port-forward -n antenna-model svc/antenna-model 3000:80
```

---

## Production Best Practices

### High Availability

1. **Multi-replica deployment** (minimum 3 replicas)
2. **Pod Anti-affinity** (spread across nodes)
3. **Pod Disruption Budget** (maintain minimum availability)
4. **Rolling updates** (zero-downtime deployments)

### Security

1. **Non-root user** (UID 1000)
2. **Read-only root filesystem** (where possible)
3. **Network policies** (restrict traffic)
4. **Secrets management** (use Kubernetes Secrets for sensitive data)
5. **RBAC** (service account with minimal permissions)

### Resource Management

1. **Set resource requests and limits**
   - CPU: 250m request, 1000m limit
   - Memory: 256Mi request, 512Mi limit
2. **Enable HPA** for automatic scaling
3. **Monitor resource usage** and adjust as needed

### Observability

1. **Structured logging** (JSON format)
2. **Request IDs** for tracing
3. **Health checks** properly configured
4. **Prometheus metrics** enabled
5. **Distributed tracing** (future enhancement)

### Disaster Recovery

1. **Backup calibration data** regularly
2. **Version control** for manifests/charts
3. **Test rollback procedures**
4. **Document incident response**

### Performance Optimization

1. **Use `fast` integration mode** for heatmaps
2. **Enable parallel batch processing** (threshold: 5)
3. **Node affinity** for CPU-optimized nodes
4. **Resource quotas** per namespace

---

## Example Deployment Workflow

### Production Deployment

```bash
# 1. Build and push image
./scripts/build-docker.sh --tag v0.1.0
docker tag antenna-model:v0.1.0 your-registry.com/antenna-model:v0.1.0
docker push your-registry.com/antenna-model:v0.1.0

# 2. Prepare calibration data
kubectl create configmap antenna-model-calibration-data \
  --from-file=calibration_data/ \
  -n antenna-model-prod

# 3. Deploy with Helm
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model-prod \
  --create-namespace \
  --values values-prod.yaml \
  --wait \
  --timeout 5m

# 4. Verify deployment
kubectl get pods -n antenna-model-prod
kubectl logs -n antenna-model-prod -l app.kubernetes.io/name=antenna-model

# 5. Test health endpoint
kubectl port-forward -n antenna-model-prod svc/antenna-model 3000:80
curl http://localhost:3000/health

# 6. Run smoke tests
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{"antenna_id":"antenna_1","feed_id":"primary","position":{"x":0,"y":0,"z":0},"frequency_hz":8.0e9}'
```

### Rolling Update

```bash
# Update to new version
helm upgrade antenna-model ./helm/antenna-model \
  --namespace antenna-model-prod \
  --set image.tag=v0.2.0 \
  --wait

# Monitor rollout
kubectl rollout status deployment/antenna-model -n antenna-model-prod

# Rollback if needed
helm rollback antenna-model -n antenna-model-prod
```

---

## Additional Resources

- **Architecture Documentation:** `docs/architecture.md`
- **Operational Runbooks:** `docs/operations/` (Sprint 8.3)
- **API Documentation:** `openapi.yaml`
- **Load Testing Guide:** `tests/load/README.md`
- **Performance Results:** `docs/performance-results.md`

---

**Document Status:** Complete
**Maintenance:** Update this document when deployment configurations change
**Owner:** DevOps/SRE Team

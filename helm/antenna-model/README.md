# Antenna Model Helm Chart

Helm chart for deploying the Antenna Model Service to Kubernetes.

## Features

- **High Availability:** Multi-replica deployment with pod anti-affinity
- **Auto-scaling:** Horizontal Pod Autoscaler (HPA) based on CPU/memory
- **Zero-downtime:** Rolling updates with PodDisruptionBudget
- **Health Checks:** Liveness and readiness probes
- **Flexible Configuration:** Environment-specific values files
- **Security:** Non-root user, security contexts, RBAC
- **Monitoring:** Prometheus metrics support (optional)
- **Ingress:** NGINX/Traefik ingress support (optional)
- **Persistence:** PVC support for calibration data (optional)

## Quick Start

### Install

```bash
# Default installation
helm install antenna-model . \
  --namespace antenna-model \
  --create-namespace

# With custom image
helm install antenna-model . \
  --namespace antenna-model \
  --create-namespace \
  --set image.repository=your-registry.com/antenna-model \
  --set image.tag=v0.1.0
```

### Upgrade

```bash
helm upgrade antenna-model . \
  --namespace antenna-model \
  --set image.tag=v0.2.0
```

### Uninstall

```bash
helm uninstall antenna-model -n antenna-model
```

## Configuration

### Key Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `replicaCount` | `2` | Number of pod replicas |
| `image.repository` | `antenna-model` | Docker image repository |
| `image.tag` | `v0.1.0` | Docker image tag |
| `service.type` | `ClusterIP` | Kubernetes service type |
| `service.port` | `80` | Service port |
| `autoscaling.enabled` | `true` | Enable HPA |
| `autoscaling.minReplicas` | `2` | Minimum replicas |
| `autoscaling.maxReplicas` | `10` | Maximum replicas |
| `resources.limits.cpu` | `1000m` | CPU limit |
| `resources.limits.memory` | `512Mi` | Memory limit |
| `ingress.enabled` | `false` | Enable ingress |
| `persistence.enabled` | `false` | Enable persistent volume |

### Environment-Specific Values

The chart includes pre-configured environment profiles in `values.yaml`:

#### Development

```bash
helm install antenna-model . \
  --namespace antenna-model-dev \
  --create-namespace \
  --set replicaCount=1 \
  --set resources.limits.cpu=500m \
  --set resources.limits.memory=256Mi \
  --set autoscaling.enabled=false \
  --set config.logging.level=debug
```

#### Staging

```bash
helm install antenna-model . \
  --namespace antenna-model-staging \
  --create-namespace \
  --set replicaCount=2 \
  --set config.logging.level=debug
```

#### Production

```bash
helm install antenna-model . \
  --namespace antenna-model-prod \
  --create-namespace \
  --set replicaCount=3 \
  --set autoscaling.minReplicas=3 \
  --set autoscaling.maxReplicas=20 \
  --set podDisruptionBudget.minAvailable=2 \
  --set config.logging.level=info
```

### Custom Values File

Create `values-prod.yaml`:

```yaml
replicaCount: 3

image:
  repository: your-registry.com/antenna-model
  tag: v0.1.0

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 20

ingress:
  enabled: true
  className: nginx
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
  size: 5Gi
```

Install with custom values:

```bash
helm install antenna-model . \
  --namespace antenna-model-prod \
  --create-namespace \
  --values values-prod.yaml
```

## Configuration Parameters

### Global

- `nameOverride` - Override chart name
- `fullnameOverride` - Override full name
- `imagePullSecrets` - Image pull secrets

### Image

- `image.repository` - Docker image repository
- `image.pullPolicy` - Image pull policy (IfNotPresent, Always)
- `image.tag` - Image tag (defaults to chart appVersion)

### Service Account

- `serviceAccount.create` - Create service account (default: true)
- `serviceAccount.annotations` - Annotations for service account
- `serviceAccount.name` - Service account name

### Service

- `service.type` - Service type (ClusterIP, LoadBalancer)
- `service.port` - Service port (default: 80)
- `service.annotations` - Service annotations

### Ingress

- `ingress.enabled` - Enable ingress (default: false)
- `ingress.className` - Ingress class (nginx, traefik)
- `ingress.annotations` - Ingress annotations
- `ingress.hosts` - Ingress hosts and paths
- `ingress.tls` - TLS configuration

### Resources

- `resources.limits.cpu` - CPU limit (default: 1000m)
- `resources.limits.memory` - Memory limit (default: 512Mi)
- `resources.requests.cpu` - CPU request (default: 250m)
- `resources.requests.memory` - Memory request (default: 256Mi)

### Autoscaling

- `autoscaling.enabled` - Enable HPA (default: true)
- `autoscaling.minReplicas` - Minimum replicas (default: 2)
- `autoscaling.maxReplicas` - Maximum replicas (default: 10)
- `autoscaling.targetCPUUtilizationPercentage` - CPU target (default: 70)
- `autoscaling.targetMemoryUtilizationPercentage` - Memory target (default: 80)

### Pod Disruption Budget

- `podDisruptionBudget.enabled` - Enable PDB (default: true)
- `podDisruptionBudget.minAvailable` - Minimum available pods (default: 1)

### Health Probes

- `livenessProbe` - Liveness probe configuration
- `readinessProbe` - Readiness probe configuration

### Persistence

- `persistence.enabled` - Enable persistent volume (default: false)
- `persistence.storageClass` - Storage class name
- `persistence.accessMode` - Access mode (default: ReadOnlyMany)
- `persistence.size` - Volume size (default: 1Gi)

### Application Configuration

All application settings under `config.*`:

```yaml
config:
  service:
    port: 3000
    request_timeout_seconds: 30
    max_request_body_size_bytes: 10485760
  logging:
    level: "info"
    format: "json"
  computation:
    default_integration_mode: "default"
    parallel_batch_threshold: 5
    max_batch_size: 1000
```

## Testing

### Verify Installation

```bash
# Check release status
helm status antenna-model -n antenna-model

# List all resources
helm get all antenna-model -n antenna-model

# Check pods
kubectl get pods -n antenna-model

# Check logs
kubectl logs -n antenna-model -l app.kubernetes.io/name=antenna-model
```

### Port Forward and Test

```bash
# Port forward
kubectl port-forward -n antenna-model svc/antenna-model 3000:80

# Test endpoints
curl http://localhost:3000/health
curl http://localhost:3000/ready
curl http://localhost:3000/status
```

### Smoke Test

```bash
# Test gain computation
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "feed_id": "primary",
    "position": {"x": 0, "y": 0, "z": 0},
    "frequency_hz": 8.0e9
  }'
```

## Troubleshooting

### Check Helm Release

```bash
helm list -n antenna-model
helm status antenna-model -n antenna-model
```

### View Logs

```bash
kubectl logs -n antenna-model -l app.kubernetes.io/name=antenna-model --tail=100 -f
```

### Describe Resources

```bash
kubectl describe deployment -n antenna-model
kubectl describe pod -n antenna-model
```

### Common Issues

**Pods not starting:**
- Check image pull: `kubectl describe pod -n antenna-model <pod-name>`
- Verify image repository and tag
- Check resource limits vs. node capacity

**Service not accessible:**
- Verify service: `kubectl get svc -n antenna-model`
- Check endpoints: `kubectl get endpoints -n antenna-model`
- Test from within cluster: `kubectl run -it debug --image=curlimages/curl -- curl http://antenna-model/health`

## Templates

The chart includes the following templates:

- `deployment.yaml` - Main application deployment
- `service.yaml` - ClusterIP and optional LoadBalancer
- `configmap.yaml` - Configuration and calibration data
- `serviceaccount.yaml` - Service account
- `pdb.yaml` - Pod disruption budget
- `hpa.yaml` - Horizontal pod autoscaler
- `ingress.yaml` - Ingress (optional)
- `pvc.yaml` - Persistent volume claim (optional)
- `_helpers.tpl` - Template helper functions
- `NOTES.txt` - Installation notes

## Further Documentation

- **Deployment Guide:** `../../docs/kubernetes-deployment.md`
- **Architecture:** `../../docs/architecture.md`
- **API Documentation:** `../../openapi.yaml`

## Chart Metadata

- **Version:** 0.1.0
- **App Version:** 0.1.0
- **Type:** application
- **Kubernetes Version:** 1.25+

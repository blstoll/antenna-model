# Deployment Quick Start Guide

This guide provides quick deployment instructions for the Antenna Model Service.

## Prerequisites

- Docker
- Kubernetes cluster (v1.25+)
- Helm v3.10+ (for Helm deployment)
- kubectl configured to access your cluster

## Option 1: Helm Deployment (Recommended)

### Development

```bash
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model-dev \
  --create-namespace \
  --set replicaCount=1 \
  --set autoscaling.enabled=false \
  --set config.logging.level=debug
```

### Production

```bash
# Build and push image
./scripts/build-docker.sh --tag v0.1.0
docker tag antenna-model:v0.1.0 your-registry.com/antenna-model:v0.1.0
docker push your-registry.com/antenna-model:v0.1.0

# Deploy with Helm
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model-prod \
  --create-namespace \
  --set image.repository=your-registry.com/antenna-model \
  --set image.tag=v0.1.0 \
  --set replicaCount=3 \
  --set autoscaling.minReplicas=3 \
  --set autoscaling.maxReplicas=20
```

## Option 2: Raw Kubernetes Manifests

```bash
# Update image in k8s/deployment.yaml first
kubectl create namespace antenna-model
kubectl apply -f k8s/ -n antenna-model
```

## Verify Deployment

```bash
# Check pods
kubectl get pods -n antenna-model

# Check logs
kubectl logs -n antenna-model -l app=antenna-model

# Port forward and test
kubectl port-forward -n antenna-model svc/antenna-model 3000:80
curl http://localhost:3000/health
```

## Test API

```bash
# Health check
curl http://localhost:3000/health

# Gain computation
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "feed_id": "primary",
    "position": {"x": 0, "y": 0, "z": 0},
    "frequency_hz": 8.0e9
  }'
```

## Full Documentation

- **Comprehensive Kubernetes Guide:** [docs/kubernetes-deployment.md](docs/kubernetes-deployment.md)
- **Helm Chart Documentation:** [helm/antenna-model/README.md](helm/antenna-model/README.md)
- **Raw Manifests:** [k8s/README.md](k8s/README.md)
- **Architecture:** [docs/architecture.md](docs/architecture.md)
- **API Reference:** [openapi.yaml](openapi.yaml)

## Troubleshooting

See [docs/kubernetes-deployment.md](docs/kubernetes-deployment.md#troubleshooting) for detailed troubleshooting guide.

## Next Steps

1. Configure ingress for external access
2. Set up monitoring and alerting
3. Configure persistent storage for calibration data
4. Review security and RBAC policies

See Sprint 8 tasks in [docs/implementation-plan.md](docs/implementation-plan.md) for operational documentation.

// docker-bake.hcl — build both Docker targets in parallel, sharing the builder stage.
// Usage: docker buildx bake --load

variable "TAG_PREFIX" {
  default = "ghcr.io/limlabs/rex"
}

variable "TAG_SUFFIX" {
  default = "ci"
}

group "default" {
  targets = ["app-build", "runtime"]
}

target "app-build" {
  dockerfile = "Dockerfile"
  tags       = ["${TAG_PREFIX}:${TAG_SUFFIX}-build"]
  target     = "app-build"
  cache-from = ["type=gha,scope=docker"]
  cache-to   = ["type=gha,scope=docker,mode=max"]
  output     = ["type=docker"]
  secret     = [
    "id=ACTIONS_CACHE_URL,env=ACTIONS_CACHE_URL",
    "id=ACTIONS_RUNTIME_TOKEN,env=ACTIONS_RUNTIME_TOKEN"
  ]
}

target "runtime" {
  dockerfile = "Dockerfile"
  tags       = ["${TAG_PREFIX}:${TAG_SUFFIX}"]
  cache-from = ["type=gha,scope=docker"]
  output     = ["type=docker"]
  secret     = [
    "id=ACTIONS_CACHE_URL,env=ACTIONS_CACHE_URL",
    "id=ACTIONS_RUNTIME_TOKEN,env=ACTIONS_RUNTIME_TOKEN"
  ]
}

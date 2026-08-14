.PHONY: release-build-env release-dist release-images release

release-build-env:
	docker build --platform linux/amd64 -f release/build-env.Dockerfile -t lightspeed-release-build .

release-dist: release-build-env
	LIGHTSPEED_RELEASE_BUILD_IMAGE="local/lightspeed-release-build@$$(docker image inspect lightspeed-release-build --format '{{.Id}}')" \
	  scripts/release/run-build-env.sh lightspeed-release-build

release-images:
	bash scripts/release/build-images.sh

release: release-dist release-images

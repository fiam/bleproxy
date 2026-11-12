# Changelog

## 0.1.0 (2026-11-12)


### Features

* **bleproxy:** add a BLE Scanning switch ([bdbbedb](https://github.com/fiam/bleproxy/commit/bdbbedb7b5c6dd76de9b204dee9db4e425ee46da))
* **bleproxy:** add a passive scan mode ([0189ff8](https://github.com/fiam/bleproxy/commit/0189ff88c4e851775738c717587e45c37b705f64))
* **bleproxy:** add host CLI entrypoint ([f5463b8](https://github.com/fiam/bleproxy/commit/f5463b8a31966f92568ca2a2bb12aad3f53f04b0))
* **bleproxy:** announce the proxy over mDNS ([92358e0](https://github.com/fiam/bleproxy/commit/92358e05d0375ffabc97c3150c2e1865cc311669))
* **bleproxy:** connect to devices through BlueZ ([78ea1ad](https://github.com/fiam/bleproxy/commit/78ea1ad9863bf47a4277dabb4c2558a5eeb28f86))
* **bleproxy:** encrypt the API with ESPHome's Noise protocol ([5854586](https://github.com/fiam/bleproxy/commit/585458644dd1dbdfeb5f573ecf686bd0b6c9e2b4))
* **bleproxy:** let Home Assistant follow and switch the scan mode ([fa0d8d8](https://github.com/fiam/bleproxy/commit/fa0d8d8787a22c1690f8b04a3f69010a6dc99481))
* **bleproxy:** limit the proxy to chosen devices ([a0ce72b](https://github.com/fiam/bleproxy/commit/a0ce72bf7ab6f94d8b66e43a8f2aeb7287d613b6))
* **bleproxy:** name unlabeled devices after their Bluetooth name ([a047f96](https://github.com/fiam/bleproxy/commit/a047f96d0a5ca49faa491dfc24da0683189342c5))
* **bleproxy:** report diagnostic sensors ([bca5613](https://github.com/fiam/bleproxy/commit/bca56138beac96552d98eb1e79c586ae4caf865d))
* **bleproxy:** report signal and last seen for allowed devices ([8561c19](https://github.com/fiam/bleproxy/commit/8561c196096e7d8e4bb77c90db4eead1e02a8f71))
* **bleproxy:** route connection requests to a connection manager ([2f86c6a](https://github.com/fiam/bleproxy/commit/2f86c6a88ad3f89424792161bcf616f64909bbdd))
* **bleproxy:** scan for advertisements with BlueZ on Linux ([edf9b58](https://github.com/fiam/bleproxy/commit/edf9b58cb8653abdebad678ecf222132b5bee929))
* **bleproxy:** serve the ESPHome Native API ([f894893](https://github.com/fiam/bleproxy/commit/f89489338cff92b9604219255b51930b3ed5a21a))
* **bleproxy:** stream logs to subscribed clients ([2f5363a](https://github.com/fiam/bleproxy/commit/2f5363ae39fdda36c902bc5dd623ad32ac9dead4))
* **core:** add advertisement and address-type types ([ce7b82f](https://github.com/fiam/bleproxy/commit/ce7b82f2e7b71291adbfacac36e581b25c3fc6a3))
* **core:** add Bluetooth UUIDs ([7f7c0c0](https://github.com/fiam/bleproxy/commit/7f7c0c0e3441272159ba3007e54baa773b8ee8fb))
* **core:** add the ESPHome API session state machine ([c2a2117](https://github.com/fiam/bleproxy/commit/c2a21177db0dbd8260674a2baf08b8d547e9b8fb))
* **core:** build advertising data from parsed fields ([8a02ff6](https://github.com/fiam/bleproxy/commit/8a02ff6a35370d66bd85cf05727920e07ae6b25e))
* **core:** hold combined advertising and scan response data ([ed10766](https://github.com/fiam/bleproxy/commit/ed107665506e36d7a51a485ac3eee001b218ac2e))
* **core:** let sessions report scanner state and switch scan modes ([dfe44f1](https://github.com/fiam/bleproxy/commit/dfe44f197932e7dd09066ec0eef055bf999bf274))
* **core:** let the platform list entities and report their states ([030c247](https://github.com/fiam/bleproxy/commit/030c2476714892b9d2d7369e8c4c11e0dedfe2ef))
* **core:** pass connection requests from sessions to the platform ([ad1d093](https://github.com/fiam/bleproxy/commit/ad1d093adceb2a352afb79d98ee13f4fc9136c78))
* **core:** pass log subscriptions to the platform ([846233b](https://github.com/fiam/bleproxy/commit/846233b3f859992907ca0199d60e23ed70986d3f))
* **core:** pass switch commands to the platform ([6ea8c2b](https://github.com/fiam/bleproxy/commit/6ea8c2b1de71ee98dba35c013adf1186e605b774))
* **core:** report encryption and encode replies for any transport ([74a353b](https://github.com/fiam/bleproxy/commit/74a353bef344f485c59d5c4de26f7678406477f5))
* **deploy:** add a podman quadlet and AppArmor profile ([fe8a469](https://github.com/fiam/bleproxy/commit/fe8a469573671075598d9fd31e53fd9e495217ca))
* **deploy:** add systemd and Docker Compose deployment files ([6d8a90e](https://github.com/fiam/bleproxy/commit/6d8a90ecde1ef22ea9a8bdab962760fcbed52562))
* **esp32:** add ESP32-C3/S3 firmware entrypoint ([7477ce4](https://github.com/fiam/bleproxy/commit/7477ce448b184ac2a21aeb0af5107c759ba8a9e4))
* **protocol:** add active connection and GATT messages ([3765927](https://github.com/fiam/bleproxy/commit/37659272975084081538af3ad8c6a4d14461bebf))
* **protocol:** add device classes and missing states ([d533dac](https://github.com/fiam/bleproxy/commit/d533dac7c1e00a291520173084f02c84b37ec9ba))
* **protocol:** add ESPHome API messages for a passive Bluetooth proxy ([7164782](https://github.com/fiam/bleproxy/commit/71647823a0b533ff38601f89eb2a5459b1ebf2aa))
* **protocol:** add ESPHome plaintext frame header codec ([ac59335](https://github.com/fiam/bleproxy/commit/ac593358b137c769dd56ab6371218d11f4546aba))
* **protocol:** add log subscription messages ([2a6674e](https://github.com/fiam/bleproxy/commit/2a6674ee1c86a0579177377b5e28132cf283e47b))
* **protocol:** add Noise framing for encrypted connections ([d212348](https://github.com/fiam/bleproxy/commit/d212348a99af3e0aff071c6eba931f73c595f256))
* **protocol:** add protobuf wire format primitives ([c2cde76](https://github.com/fiam/bleproxy/commit/c2cde760cbb1795ab50f19f2d92572ee9fb6915a))
* **protocol:** add scanner state and mode messages ([fd4c5a5](https://github.com/fiam/bleproxy/commit/fd4c5a5ea580fb8a514a2721b08fd9848ba1301c))
* **protocol:** add sensor and text sensor messages ([c676494](https://github.com/fiam/bleproxy/commit/c676494b450c8a750407da411d67132b3794f14f))
* **protocol:** add switch messages ([49d42f0](https://github.com/fiam/bleproxy/commit/49d42f0ff67f711d49815203fac71f60388000e9))
* **protocol:** encode complete plaintext frames ([9a1a894](https://github.com/fiam/bleproxy/commit/9a1a894c40caabe7e459384c4e1a76ee321f540a))


### Bug Fixes

* **bleproxy:** count IPv4 clients of an IPv6 socket once ([e48dfca](https://github.com/fiam/bleproxy/commit/e48dfca9ac21b04bdda5f1180d45db80a755254b))
* **bleproxy:** only announce addresses the API listens on ([2302039](https://github.com/fiam/bleproxy/commit/230203982abc2fa26a78a7ee3e4184957444c17b))
* **bleproxy:** ping quiet clients instead of dropping them ([72c5570](https://github.com/fiam/bleproxy/commit/72c5570b2a970312e4a75863dc7e7a12e54e8c5e))
* **bleproxy:** scan actively when BlueZ can't scan passively ([06d3e34](https://github.com/fiam/bleproxy/commit/06d3e34e76de993844c9756fdeffb66dd6a411be))
* **bleproxy:** use icons that exist in Material Design Icons ([3383178](https://github.com/fiam/bleproxy/commit/33831787400e49ff38b678346bb99fd90b61f03a))
* **docker:** let the container reach the host's BlueZ ([ebf7145](https://github.com/fiam/bleproxy/commit/ebf71453a812d856dbf137e2d8f0567300449203))
* **xtask:** detect cargo plugins when color output is forced ([f8c2060](https://github.com/fiam/bleproxy/commit/f8c206045b06e43577d3e0fc2d7700a9326b8740))
* **xtask:** make the metadata task detect a stale lockfile ([d8c3cbb](https://github.com/fiam/bleproxy/commit/d8c3cbbd5f5751fc912c840f99a6b0a19fe3a2fa))


### Documentation

* add AGENTS.md ([b8063ef](https://github.com/fiam/bleproxy/commit/b8063ef93a21630f810d65961a1e00c99d9ea9b4))
* add README and contributing guide ([613fbd1](https://github.com/fiam/bleproxy/commit/613fbd1bbca72379d076c76348ca3c3db488905e))
* describe allowed devices' signal sensors ([ace1304](https://github.com/fiam/bleproxy/commit/ace130459db5194a07949cdece582315d5df76b3))
* describe streaming logs to Home Assistant ([402ed87](https://github.com/fiam/bleproxy/commit/402ed879107f25ab7c88f56f70839934135814bc))
* describe the BLE Scanning switch ([b73a3cf](https://github.com/fiam/bleproxy/commit/b73a3cf65927e6442189a2a2686222b560717b24))
* describe the diagnostic sensors ([0b300a4](https://github.com/fiam/bleproxy/commit/0b300a445aaf5c6367756e9318965da56d7b8042))
* document active connections ([4a4d315](https://github.com/fiam/bleproxy/commit/4a4d31573c1524ce1d02df93060b2d845ccc1407))
* document scan mode switching and API encryption ([e37ac94](https://github.com/fiam/bleproxy/commit/e37ac9455b5da476bb5e0ef530c2f7e451610b67))
* document the address filter ([cc01288](https://github.com/fiam/bleproxy/commit/cc012887bd64b61db88cf40b3f6111ef9eb28821))
* explain how to run bleproxy on a server ([6d4de62](https://github.com/fiam/bleproxy/commit/6d4de6228f62d13ac0737146605f9bceb7333263))


### Build System

* configure cargo-deny supply-chain policy ([1347b7e](https://github.com/fiam/bleproxy/commit/1347b7e36bb6c733bfd931419ca6b3ea7ed7e053))
* declare MSRV 1.95 and verify it with cargo xtask msrv ([450a8c9](https://github.com/fiam/bleproxy/commit/450a8c996b589a868bb3b24ef232ba1116cf5511))
* **docker:** add release container image ([23ae42a](https://github.com/fiam/bleproxy/commit/23ae42a2bd69f2dcd9ae5c2f61770298ac427022))
* mark workspace crates as unpublished ([33a73b1](https://github.com/fiam/bleproxy/commit/33a73b1a9f33e927c1dfcba0ef77a144f7782459))
* set up cargo workspace, toolchain and lint policy ([c32992d](https://github.com/fiam/bleproxy/commit/c32992dc74d87120aef03819c1f5b3aa541120f3))
* **xtask:** add cargo xtask task runner ([8f9ff99](https://github.com/fiam/bleproxy/commit/8f9ff99b66159425067afc466694d3ef773e94e2))
* **xtask:** add docker-based linux and windows test runs ([c40b6f2](https://github.com/fiam/bleproxy/commit/c40b6f2e1b5a6c992747f97a83c21db5958ecfea))


### Continuous Integration

* add Dependabot for Cargo and GitHub Actions ([af0c150](https://github.com/fiam/bleproxy/commit/af0c150a4f77821afd6ee81c0d9ad1b425dd1017))
* add GitHub Actions workflow ([306877a](https://github.com/fiam/bleproxy/commit/306877a7b48c4f8dddbdd2da28bc34d1613a0f66))
* add release-please release workflow ([957e114](https://github.com/fiam/bleproxy/commit/957e114765f7519a464daec892228bbe44831dd0))
* don't hold edge images back on firmware builds ([af03bf7](https://github.com/fiam/bleproxy/commit/af03bf74c7d3297ae3c03a8808b4b2c4f58b7bab))
* make 0.1.0 the first release ([fefc4bc](https://github.com/fiam/bleproxy/commit/fefc4bccb91369cea51961a69012e974ab16d906))
* move container image publishing into a reusable workflow ([d3b1238](https://github.com/fiam/bleproxy/commit/d3b12388381b5f09d53fcac680aa63668e4ec717))
* pass a token to the Xtensa toolchain installer ([2cccb4b](https://github.com/fiam/bleproxy/commit/2cccb4bf2068b9d0011602e5110bb1c237143594))
* publish edge images from main ([5427a78](https://github.com/fiam/bleproxy/commit/5427a786cade14faf4fb25947dc8b56edf53a7dd))
* publish release binaries and multi-arch images ([3289ec0](https://github.com/fiam/bleproxy/commit/3289ec066b60a6090614a733a82b7142bb180baa))
* require conventional commit pull request titles ([6b62ac1](https://github.com/fiam/bleproxy/commit/6b62ac10da3ae1281e2bad338c3ff8d22abfd96b))
* start release-please at 0.1.0 ([3e21706](https://github.com/fiam/bleproxy/commit/3e21706f1d2ba5167265cb7c992a404add6f4b1a))

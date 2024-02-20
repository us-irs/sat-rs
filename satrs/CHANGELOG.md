Change Log
=======

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

# [unreleased]

# [v0.2.0]

## Added

- New PUS service abstractions for HK (PUS 3) and actions (PUS 8). Introducing new abstractions
  allows to move some boilerplate code into the framework.
- New `VerificationReportingProvider` abstraction to avoid relying on a concrete verification
  reporting provider.

## Changed

- Verification reporter API timestamp arguments are not `Option`al anymore. Empty timestamps
  can be passed by simply specifying the `&[]` empty slice argument.

# [v0.1.1] 2024-02-12

- Minor fixes for crate config `homepage` entries and links in documentation.

# [v0.1.0] 2024-02-12

Initial release.

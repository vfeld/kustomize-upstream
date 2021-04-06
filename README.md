## What is this?
This repo hosts the command line tool kustomize-upstream. The goal of this tool is to ease the integration and maintenance of upstream project versions into kustomize based projects.

kustomize-upstream reads a multi-document yaml and splits it to multiple packages each containing one manifest file per manifest using user defined split rules. Split rules use the kubernetes manifest parameters kind, name or namespace as criteria. kustomize-upstream generates as well kustomization.yaml using templates.

Package metadata, split rules and the package descriptor templates are specified in a yaml configuration file passed as argument to the kustomize-upstream.

## Status
Please note, this is in alpha status 
- generates unfriendly error message
- no tests
- unstable yaml schema
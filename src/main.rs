use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env::args;
use std::fs;
use std::io::{Read};
use std::path::Path;
use tera::{Context, Tera};
use yaml_merge_keys::merge_keys;
use yaml_rust::{Yaml, YamlEmitter, YamlLoader};

#[allow(non_snake_case)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Config {
    Top: Top,
    DefaultPackageSpec: DefaultPackageSpec,
    SplitRules: Vec<SplitRule>,
}
#[allow(non_snake_case)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Top {
    name: String,
    version: String,
    sourceTemplate: String,
    source: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct DefaultPackageSpec {
    template: String,
    defaultName: String,
    filenameTemplate: String,
    pathTemplate: String,
    resourceSpec: ResourceSpec,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ResourceSpec {
    pathTemplate: String,
    filenameTemplate: String,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SplitRule {
    matcher: Matcher,
    packageName: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
struct Matcher {
    kind: Option<String>,
    name: Option<String>,
    namespace: Option<String>,
}

#[derive(Clone, Serialize)]
struct Package {
    name: String,
    resources: Vec<Resource>,
}

#[derive(Clone, Serialize, PartialEq)]
struct Resource {
    index: u32,
    name: String,
    kind: String,
    namespace: Option<String>,
    filename: Option<String>,
    path: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if args().len() != 2 {
        println!("
usage: kustomize-upstream <config.yaml>

kustomize-upstream reads a multi-document 
yaml and splits it to multiple packages 
each containing one manifest file per manifest 
using user defined split rules. Split rules 
use the kubernetes manifest parameters kind, 
name or namespace as criteria. kustomize-upstream 
generates as well kustomization.yaml using 
templates.

config.yaml example:

Top:
  name: contour
  version: 1.14.0
  sourceTemplate: https://raw.githubusercontent.com/projectcontour/contour/v{{{{top.version}}}}/examples/render/contour.yaml
DefaultPackageSpec:
  template: | 
    apiVersion: kustomize.config.k8s.io/v1beta1
    kind: Kustomization
    resources:
      {{% for resource in package.resources -%}}
      - {{{{resource.filename}}}}
      {{% endfor -%}}
  pathTemplate: {{{{top.name}}}}-{{{{top.version}}}}/{{{{packageName}}}}
  filenameTemplate: kustomization.yaml
  defaultName: main
  resourceSpec:
    pathTemplate: {{{{top.name}}}}-{{{{top.version}}}}/{{{{packageName}}}}
    filenameTemplate: {{{{ '%03d' % resource.index}}}}_{{{{resource.kind}}}}_{{{{resource.name}}}}.yaml
SplitRules:
  - matcher:
      kind: clusterrole
    packageName: cr
  - matcher:
      kind: clusterrolebinding
    packageName: crb
  - matcher:
      kind: customresourcedefinition
    packageName: crd 
");
        std::process::exit(exitcode::CONFIG);
    }
    let config_path = args().nth(1).unwrap();
    let config_yaml = fs::read_to_string(config_path).unwrap();
    let mut config: Config = serde_yaml::from_str(&config_yaml).unwrap();

    let mut idx = 0u32;
    let mut packages: HashMap<String, Package> = HashMap::new();
    let source = config.render_source();
    config.Top.source = Some(source.clone());

    let mut resp = reqwest::blocking::get(source).unwrap();
    if resp.status() != reqwest::StatusCode::OK {
        println!("unable to fetch the upstream project");
        std::process::exit(exitcode::UNAVAILABLE);
    }

    let mut manifests_yaml = String::new();
    //io::stdin().read_to_string(&mut manifests_yaml)?;
    resp.read_to_string(&mut manifests_yaml)?;
    let manifests = YamlLoader::load_from_str(&manifests_yaml).unwrap();

    for manifest in manifests {
        let manifest = merge_keys(manifest).unwrap();

        //get resource metadata
        let mut resource = if let Some(resource) = Resource::from_manifest(&manifest, idx) {
            idx += 1;
            resource
        } else {
            idx += 1;
            continue;
        };

        //classify resource and store resource per package

        let package_name = match config.classify(&resource) {
            Some(package_name) => package_name,
            None => continue,
        };
        let package = match packages.get_mut(&package_name) {
            Some(package) => package,
            None => {
                let c = Package {
                    name: package_name.clone(),
                    resources: Vec::new(),
                };
                packages.insert(package_name.clone(), c);
                packages.get_mut(&package_name).unwrap()
            }
        };
        let filename = config.render_resource_filename(package, &resource);
        let pathname = config.render_resource_path(package, &resource);

        resource.filename = Some(filename.clone());
        resource.path = Some(pathname.clone());

        package.resources.push(resource);

        //write resource yaml
        let path = Path::new(&pathname);
        fs::create_dir_all(&path).unwrap();
        let filepath = path.join(filename);

        let mut out_str = String::new();
        {
            let mut emitter = YamlEmitter::new(&mut out_str);
            emitter.dump(&manifest).unwrap(); // dump the YAML object to a String
        }
        println!("create file: {}", filepath.display().to_string());
        fs::write(filepath.display().to_string(), out_str).expect("Unable to write file");
    }
    // write package descriptor for each package
    for (_package_name, package) in packages {
        let pathname = config.render_package_path(&package);
        let filename = config.render_package_filename(&package);
        let path = Path::new(&pathname);
        let filepath = path.join(filename);
        let package_yaml = config.render_package_descriptor(&package);
        println!("create file: {}", filepath.display().to_string());
        fs::write(filepath.display().to_string(), package_yaml).expect("Unable to write file");
    }
    return Ok(());
}

impl Config {
    fn classify(&self, resource: &Resource) -> Option<String> {
        for rule in &self.SplitRules {
            if rule.matcher.do_match(resource) {
                return rule.packageName.clone();
            }
        }

        let package_name = self.DefaultPackageSpec.defaultName.clone();
        Some(package_name)
    }

    fn render_package_descriptor(&self, package: &Package) -> String {
        let mut context = Context::new();
        context.insert("top", &self.Top);
        context.insert("package", &package);
        let mut tera = Tera::default();
        tera.register_filter("pad3", Pad3Fn {});
        tera.add_raw_templates(vec![(
            "DefaultPackageSpec.template",
            self.DefaultPackageSpec.template.clone(),
        )])
        .unwrap();
        let package_yaml = tera
            .render("DefaultPackageSpec.template", &context)
            .unwrap();
        package_yaml
    }

    fn render_source(&self) -> String {
        let mut context = Context::new();
        context.insert("top", &self.Top);
        let mut tera = Tera::default();
        tera.register_filter("pad3", Pad3Fn {});
        tera.add_raw_templates(vec![("Top.sourceTemplate", &self.Top.sourceTemplate)])
            .unwrap();
        let source = tera.render("Top.sourceTemplate", &context).unwrap();
        source
    }

    fn render_resource_filename(&self, package: &Package, resource: &Resource) -> String {
        let mut context = Context::new();
        context.insert("top", &self.Top);
        context.insert("packageName", &package.name);
        context.insert("resource", &resource);

        let mut tera = Tera::default();
        tera.register_filter("pad3", Pad3Fn {});
        tera.add_raw_templates(vec![(
            "DefaultPackageSpec.resourceSpec.filenameTemplate",
            self.DefaultPackageSpec
                .resourceSpec
                .filenameTemplate
                .clone(),
        )])
        .unwrap();
        tera.render("DefaultPackageSpec.resourceSpec.filenameTemplate", &context)
            .unwrap()
    }

    fn render_resource_path(&self, package: &Package, resource: &Resource) -> String {
        let mut context = Context::new();
        context.insert("top", &self.Top);
        context.insert("packageName", &package.name);
        context.insert("resource", &resource);

        let mut tera = Tera::default();
        tera.register_filter("pad3", Pad3Fn {});

        tera.add_raw_templates(vec![(
            "DefaultPackageSpec.resourceSpec.pathTemplate",
            self.DefaultPackageSpec.resourceSpec.pathTemplate.clone(),
        )])
        .unwrap();
        tera.render("DefaultPackageSpec.resourceSpec.pathTemplate", &context)
            .unwrap()
    }
    fn render_package_filename(&self, package: &Package) -> String {
        let mut context = Context::new();
        context.insert("top", &self.Top);
        context.insert("packageName", &package.name);

        let mut tera = Tera::default();
        tera.register_filter("pad3", Pad3Fn {});

        tera.add_raw_templates(vec![(
            "DefaultPackageSpec.filenameTemplate",
            self.DefaultPackageSpec.filenameTemplate.clone(),
        )])
        .unwrap();
        tera.render("DefaultPackageSpec.filenameTemplate", &context)
            .unwrap()
    }

    fn render_package_path(&self, package: &Package) -> String {
        let mut context = Context::new();
        context.insert("top", &self.Top);
        context.insert("packageName", &package.name);

        let mut tera = Tera::default();
        tera.register_filter("pad3", Pad3Fn {});

        tera.add_raw_templates(vec![(
            "DefaultPackageSpec.pathTemplate",
            self.DefaultPackageSpec.pathTemplate.clone(),
        )])
        .unwrap();
        tera.render("DefaultPackageSpec.pathTemplate", &context)
            .unwrap()
    }
}

impl Matcher {
    fn do_match(&self, resource: &Resource) -> bool {
        if self.kind != None {
            if self.kind.clone().unwrap().to_lowercase() != resource.kind.to_lowercase() {
                return false;
            }
        }
        if self.name != None {
            if self.name.clone().unwrap().to_lowercase() != resource.name.to_lowercase() {
                return false;
            }
        }
        if self.namespace != None {
            if self.namespace.clone().map(|s| s.to_lowercase())
                != resource.clone().namespace.map(|s| s.to_lowercase())
            {
                return false;
            }
        }
        return true;
    }
}

impl Resource {
    fn from_manifest(manifest: &Yaml, idx: u32) -> Option<Resource> {
        let kind = if let Some(kind) = manifest["kind"].as_str() {
            kind
        } else {
            return None;
        };
        let name = manifest["metadata"]["name"].as_str().unwrap();
        let namespace = manifest["metadata"]["namespace"]
            .as_str()
            .map(|s| s.to_string());

        let resource = Resource {
            index: idx,
            name: name.to_string(),
            kind: kind.to_string(),
            namespace: namespace,
            filename: None,
            path: None,
        };
        Some(resource)
    }
}

struct Pad3Fn {}

impl tera::Filter for Pad3Fn {
    fn filter(
        &self,
        value: &tera::Value,
        _args: &HashMap<String, tera::Value>,
    ) -> tera::Result<tera::Value> {
        match value {
            tera::Value::Number(num) => {
                if let Some(num) = num.as_u64() {
                    let result = format!("{:03}", num);
                    Ok(tera::Value::String(result))
                } else {
                    Err("expect number".into())
                }
            }
            _ => Err("expect number".into()),
        }
    }
}

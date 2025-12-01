# usage example:
# ./getclosure.sh 127.0.0.1:8080
# ./getclosure.sh 127.0.0.1:8080 /nix/store/lrrywp3k594k3295lh92lm7a387wk0j9-hello-2.12.1.drv

# ref https://github.com/NixOS/nix/issues/2712
# substituted toplevel outPaths have a unknown-deriver `nix-store --query --deriver``
# collecting a closure will need the top level derivation
# "it's not gone it's just not referenced by the outPath"
# this might be a source for errors at some point
get_unknown_deriver() {
  local outPath="$1";
  nix derivation show "$outPath" |
  jq 'with_entries(. as {key:$k, value:$v} | {value: $k, key: ($ARGS.positional[] | select(startswith($k, $v.outputs[].path)))})' --args "$outPath"
}

# check if required environment variables are present
required_vars=(
  CI_MERGE_REQUEST_PROJECT_ID
  CI_MERGE_REQUEST_IID
  CI_COMMIT_SHA
  CI_JOB_NAME
  CI_PIPELINE_ID
)
for var in "${required_vars[@]}"; do
  if [ -z "${!var}" ]; then
    echo "Environment variable '$var' is necessary"
    if [[ "$CI_COMMIT_BRANCH" == "master" || "$CI_COMMIT_BRANCH" == "main" ]]; then
      echo "this is main or master branch, no reproducibility tests are done"
      exit 0
    fi
    exit 1
  fi
done


# manually give a store derivation as $2 to use instead of symlink from result
if [ -n "$2" ]; then
    # ref https://github.com/NixOS/nix/issues/7562
    # used for testing purposes
    store_derivation=$2
    echo "derivation (arg): $store_derivation"
else
    store_derivation=$(nix-store --query --deriver result)
    echo "derivation (query nix-store): $store_derivation"

    # a substituted toplevel might not link correctly to store derivation
    if [[ "$store_derivation" == "unknown-deriver" ]]; then
        outPath=$(readlink -f result)
        store_derivation=$(get_unknown_deriver $outPath | jq -r --arg outPath "${outPath}" '.[$outPath]')
        echo "derivation (getter function): $store_derivation"
    fi
fi

# collect the build closure
store_derivation_closure=$(nix-store --query --requisites "$store_derivation")
path=$(pwd)

closure_count=$(echo $store_derivation_closure | wc -w)
echo "closure elements collected: $closure_count"

jsonfile="$path/tmp_closure-paths.json"
closure_paths_json=$(echo "$store_derivation_closure" | nix run nixpkgs#jq -- -R -s -c 'split("\n") | map(select(length > 0))')
echo "$closure_paths_json" > "$jsonfile"

# build json metadata for the test request
json_body=$(nix run nixpkgs#jq -- -n \
  --arg store_derivation "$store_derivation" \
  --slurpfile closure_paths "$jsonfile" \
  --arg ci_merge_request_project_id "$CI_MERGE_REQUEST_PROJECT_ID" \
  --arg ci_merge_request_iid "$CI_MERGE_REQUEST_IID" \
  --arg ci_commit_sha "$CI_COMMIT_SHA" \
  --arg ci_job_name "$CI_JOB_NAME" \
  --arg ci_pipeline_id "$CI_PIPELINE_ID" \
  '{store_derivation: $store_derivation, store_derivation_closure: $closure_paths[0], ci_merge_request_project_id: $ci_merge_request_project_id, ci_merge_request_iid: $ci_merge_request_iid, ci_commit_sha: $ci_commit_sha, ci_job_name: $ci_job_name, ci_pipeline_id: $ci_pipeline_id}')
echo "$json_body" > "$jsonfile"

echo "preparing multipart: json is created"

# create a partial binary stream export of all nix derivations necessary
file="tmp_nix-export"
filepath="$path/$file"
nix-store --export $store_derivation_closure > "$filepath"

echo "preparing multipart: closure is exported"

response=$(curl -s -X POST \
-F "json=@$jsonfile" \
-F "closure=@$filepath" \
"http://$1/report")

#rm "$filepath"
#rm "$jsonfile"

if [[ "$response" == "" ]]; then
    echo "no server response"
    exit 1
fi

echo "server response: $response"
exit 0
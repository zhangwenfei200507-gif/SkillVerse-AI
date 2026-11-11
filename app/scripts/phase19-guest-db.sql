INSERT INTO entities(id,kind,name,source_type)
VALUES('phase19-guest-fixture','capability','Synthetic guest capability','manual');
INSERT INTO learning_records(id,entity_id,status)
VALUES('phase19-guest-fixture','phase19-guest-fixture','PRACTICING');

INSERT INTO workflows VALUES('phase19-guest-fixture','Synthetic completed mission','{}',1,1);
INSERT INTO workflow_versions VALUES('phase19-guest-fixture',1,'{
  "workflowId":"phase19-guest-fixture","revision":1,"goal":"Check a synthetic workspace",
  "projectRoot":"","complexity":"simple","scope":"local","goalVerifiable":false,
  "nodes":[{"id":"fixture","capabilityId":null,"capabilityType":"builtin","capabilityVersion":null,
    "description":"Synthetic completed inspection","dependencies":[],"inputSchema":{},"outputSchema":{},
    "adapter":"builtin.inspect","risk":"LOW","estimatedCost":"none","timeoutMs":1000,"maxAttempts":1,
    "retryPolicy":"none","verificationPolicy":"none","params":{},"condition":null}],"routing":{}
}','phase19-synthetic-digest',1);
INSERT INTO workflow_nodes
SELECT workflow_id, revision, 'fixture', json_extract(plan_json,'$.nodes[0]')
FROM workflow_versions WHERE workflow_id='phase19-guest-fixture';
INSERT INTO execution_runs(id,workflow_id,revision,mode,status,request_key,plan_digest,workspace,created_at,started_at,finished_at)
VALUES('phase19-guest-fixture','phase19-guest-fixture',1,'DRY_RUN','SUCCEEDED',
 'phase19-guest-fixture','phase19-synthetic-digest','',1,1,2);
INSERT INTO execution_steps(id,execution_id,node_id,attempt,status,idempotency_key,side_effect,input_digest,started_at,finished_at)
VALUES('phase19-guest-fixture','phase19-guest-fixture','fixture',1,'SUCCEEDED',
 'phase19-guest-fixture','NONE','phase19-input',1,2);
INSERT INTO execution_events(workflow_id,execution_id,node_id,event,data_json,created_at)
VALUES('phase19-guest-fixture','phase19-guest-fixture','fixture','SUCCEEDED','{}',2);

INSERT INTO ai_model_providers VALUES('phase19-guest-fixture',
 '{"id":"phase19-guest-fixture","name":"Synthetic offline provider","transport":"mock","baseUrl":"","enabled":false}',
 'phase19-fixture',1);
INSERT INTO ai_models VALUES('phase19-guest-fixture','phase19-guest-fixture',
 '{"id":"phase19-guest-fixture","providerId":"phase19-guest-fixture","modelId":"offline-fixture","enabled":false,
 "contextWindow":4096,"maxOutputTokens":128,"timeoutSeconds":5,"maxAttempts":1,"dailyRequests":1,"dailyTokens":4096,
 "dailyBudgetMicrousd":null,"taskTokens":4096,"taskBudgetMicrousd":null,"inputPricePerMillionMicrousd":null,
 "outputPricePerMillionMicrousd":null,"declaredFeatures":["text"]}',
 'phase19-fixture','{"observed":[],"source":"","checkedAt":0,"lastError":null}',0,1);
INSERT INTO ai_model_calls(id,model_id,model_revision,request_digest,day,status,owner_id,
 reserved_requests,reserved_tokens,reserved_microusd,created_at,finished_at)
VALUES('phase19-guest-fixture','phase19-guest-fixture','phase19-fixture',
 'phase19-fixture','2000-01-01','SUCCEEDED','phase19-fixture',1,1,1,1,2);
INSERT INTO ai_live_spend(request_id,provider_id,model_id,intent_digest,purpose,input_price,
 cached_price,output_price,reserved_microusd,settled_microusd)
VALUES('phase19-guest-fixture','phase19-guest-fixture','phase19-guest-fixture',
 'phase19-fixture','acceptance',0,0,0,1,0);

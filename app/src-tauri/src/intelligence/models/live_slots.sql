INSERT INTO ai_model_providers
SELECT id, json_object('id',id,'name',name,'transport','openai-compatible',
 'baseUrl',url,'enabled',json('true')), 'phase13', 0
FROM (
 SELECT 'phase13-openai' AS id, 'OpenAI' AS name, 'https://api.openai.com/v1' AS url
 UNION ALL SELECT 'phase13-deepseek','DeepSeek','https://api.deepseek.com'
);
INSERT INTO ai_models
SELECT id, provider, json_object('id',id,'providerId',provider,'modelId',model,
 'enabled',json('true'),'contextWindow',32768,'maxOutputTokens',output,'timeoutSeconds',30,
 'maxAttempts',1,'dailyRequests',100,'dailyTokens',1000000,'dailyBudgetMicrousd',50000,
 'taskTokens',32768,'taskBudgetMicrousd',20000,'inputPricePerMillionMicrousd',input_price,
 'outputPricePerMillionMicrousd',output_price,
 'declaredFeatures',json('["text","structured_output","tool_selection","reasoning"]')),
 'phase13-' || id, json_object('observed',json('[]'),'source','','checkedAt',0,'lastError',NULL),0,0
FROM (
 SELECT 'phase13-luna' AS id, 'phase13-openai' AS provider,'gpt-5.6-luna' AS model,1024 AS output,200000 AS input_price,1200000 AS output_price
 UNION ALL SELECT 'phase13-flash','phase13-deepseek','deepseek-flash',1024,300000,1200000
 UNION ALL SELECT 'phase13-pro','phase13-deepseek','deepseek-v4-pro',1536,1320000,3960000
 UNION ALL SELECT 'phase13-terra','phase13-openai','gpt-5.6-terra',1024,2000000,12000000
);

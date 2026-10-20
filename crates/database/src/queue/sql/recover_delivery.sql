UPDATE message_queue SET state = 'uncertain', error = 'Delivery was interrupted. Check the conversation before choosing Retry or Remove.' WHERE state IN ('dispatching','steering');

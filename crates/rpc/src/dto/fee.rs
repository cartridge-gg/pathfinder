use super::U256Hex;

#[derive(Debug, PartialEq, Eq)]
pub struct FeeEstimate<'a>(pub &'a pathfinder_executor::types::FeeEstimate);

impl crate::dto::serialize::SerializeForVersion for FeeEstimate<'_> {
    fn serialize(
        &self,
        serializer: crate::dto::serialize::Serializer,
    ) -> Result<crate::dto::serialize::Ok, crate::dto::serialize::Error> {
        let mut serializer = serializer.serialize_struct()?;

        if serializer.version >= crate::dto::RpcVersion::V08 {
            serializer.serialize_field("l1_gas_consumed", &U256Hex(self.0.l1_gas_consumed))?;
            serializer.serialize_field("l1_gas_price", &U256Hex(self.0.l1_gas_price))?;
            serializer.serialize_field(
                "l1_data_gas_consumed",
                &U256Hex(self.0.l1_data_gas_consumed),
            )?;
            serializer.serialize_field("l1_data_gas_price", &U256Hex(self.0.l1_data_gas_price))?;
            serializer.serialize_field("l2_gas_consumed", &U256Hex(self.0.l2_gas_consumed))?;
            serializer.serialize_field("l2_gas_price", &U256Hex(self.0.l2_gas_price))?;
            serializer.serialize_field("overall_fee", &U256Hex(self.0.overall_fee))?;
            serializer.serialize_field("unit", &PriceUnit(&self.0.unit))?;
        } else {
            serializer.serialize_field("gas_price", &U256Hex(self.0.l1_gas_price))?;
            serializer.serialize_field("gas_consumed", &U256Hex(self.0.l1_gas_consumed))?;
            serializer
                .serialize_field("data_gas_consumed", &U256Hex(self.0.l1_data_gas_consumed))?;
            serializer.serialize_field("data_gas_price", &U256Hex(self.0.l1_data_gas_price))?;
            serializer.serialize_field("overall_fee", &U256Hex(self.0.overall_fee))?;
            serializer.serialize_field("unit", &PriceUnit(&self.0.unit))?;
        }

        serializer.end()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PriceUnit<'a>(&'a pathfinder_executor::types::PriceUnit);

impl crate::dto::serialize::SerializeForVersion for PriceUnit<'_> {
    fn serialize(
        &self,
        serializer: crate::dto::serialize::Serializer,
    ) -> Result<crate::dto::serialize::Ok, crate::dto::serialize::Error> {
        serializer.serialize_str(match self.0 {
            pathfinder_executor::types::PriceUnit::Wei => "WEI",
            pathfinder_executor::types::PriceUnit::Fri => "FRI",
        })
    }
}
